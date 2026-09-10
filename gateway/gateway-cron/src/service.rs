//! # Cron Job Service
//!
//! CRUD operations and persistence for cron jobs.

use crate::config::{CreateCronJobRequest, CronJobConfig, CronJobsStore, UpdateCronJobRequest};
use agent_primitives::vault_paths::SharedVaultPaths;
use std::path::PathBuf;
use thiserror::Error;
use tokio::fs;
use tracing::{debug, info};

/// Errors from cron service operations.
#[derive(Error, Debug)]
pub enum CronServiceError {
    #[error("Cron job not found: {0}")]
    NotFound(String),

    #[error("Cron job already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid job ID: {0}")]
    InvalidId(String),

    #[error("Invalid cron schedule: {0}")]
    InvalidSchedule(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Result type for cron service operations.
pub type CronResult<T> = Result<T, CronServiceError>;

/// Service for managing cron jobs with persistence.
#[derive(Clone)]
pub struct CronService {
    paths: SharedVaultPaths,
}

impl CronService {
    /// Create a new cron service.
    pub fn new(paths: SharedVaultPaths) -> Self {
        Self { paths }
    }

    /// Get the config file path.
    fn config_path(&self) -> PathBuf {
        self.paths.cron_jobs()
    }

    /// Load cron jobs from disk.
    pub async fn load(&self) -> CronResult<CronJobsStore> {
        if !self.config_path().exists() {
            debug!("Cron jobs config not found, returning empty store");
            return Ok(CronJobsStore::default());
        }

        let content = fs::read_to_string(&self.config_path()).await?;
        let store: CronJobsStore = serde_json::from_str(&content)?;
        debug!(count = store.jobs.len(), "Loaded cron jobs from disk");
        Ok(store)
    }

    /// Save cron jobs to disk.
    async fn save(&self, store: &CronJobsStore) -> CronResult<()> {
        // Ensure parent directory exists
        if let Some(parent) = self.config_path().parent() {
            fs::create_dir_all(parent).await?;
        }

        let content = serde_json::to_string_pretty(store)?;
        fs::write(&self.config_path(), content).await?;
        debug!(
            path = %self.config_path().display(),
            count = store.jobs.len(),
            "Saved cron jobs to disk"
        );
        Ok(())
    }

    /// Normalize and validate a cron schedule expression.
    ///
    /// `tokio-cron-scheduler` requires 6-field cron (`sec min hour day
    /// month weekday`) — 5-field Unix/Vixie cron is rejected. Many
    /// callers (UI presets, third-party API consumers) send 5-field;
    /// auto-prepend `0 ` so they get the natural at-second-zero
    /// interpretation. Returns the canonical 6-field form for storage so
    /// the scheduler reads it back unchanged.
    fn normalize_schedule(schedule: &str) -> CronResult<String> {
        use tokio_cron_scheduler::Job;

        let trimmed = schedule.trim();
        let field_count = trimmed.split_whitespace().count();
        let normalized = if field_count == 5 {
            format!("0 {}", trimmed)
        } else {
            trimmed.to_string()
        };

        match Job::new_async(normalized.as_str(), |_uuid, _lock| Box::pin(async {})) {
            Ok(_) => Ok(normalized),
            Err(e) => Err(CronServiceError::InvalidSchedule(format!(
                "'{}': {}",
                schedule, e
            ))),
        }
    }

    /// List all cron jobs.
    pub async fn list(&self) -> CronResult<Vec<CronJobConfig>> {
        let store = self.load().await?;
        Ok(store.jobs)
    }

    /// Get a cron job by ID.
    pub async fn get(&self, id: &str) -> CronResult<CronJobConfig> {
        let store = self.load().await?;
        store
            .jobs
            .into_iter()
            .find(|j| j.id == id)
            .ok_or_else(|| CronServiceError::NotFound(id.to_string()))
    }

    /// Create a new cron job.
    pub async fn create(&self, request: CreateCronJobRequest) -> CronResult<CronJobConfig> {
        // Validate ID
        if request.id.is_empty() {
            return Err(CronServiceError::InvalidId(
                "ID cannot be empty".to_string(),
            ));
        }

        if !request
            .id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(CronServiceError::InvalidId(format!(
                "ID '{}' contains invalid characters (only alphanumeric, -, _ allowed)",
                request.id
            )));
        }

        // Normalize 5-field cron to 6-field and validate
        let schedule = Self::normalize_schedule(&request.schedule)?;

        let mut store = self.load().await?;

        // Check for duplicates
        if store.jobs.iter().any(|j| j.id == request.id) {
            return Err(CronServiceError::AlreadyExists(request.id));
        }

        let now = chrono::Utc::now();
        let job = CronJobConfig {
            id: request.id.clone(),
            name: request.name,
            schedule,
            kind: request.kind,
            agent_id: request.agent_id,
            message: request.message,
            http: request.http,
            respond_to: request.respond_to,
            enabled: request.enabled,
            timezone: request.timezone,
            metadata: request.metadata,
            last_run: None,
            next_run: None, // Will be calculated by scheduler
            created_at: Some(now),
            updated_at: Some(now),
        };

        store.jobs.push(job.clone());
        self.save(&store).await?;

        info!(job_id = %job.id, "Created cron job");
        Ok(job)
    }

    /// Update an existing cron job.
    pub async fn update(
        &self,
        id: &str,
        request: UpdateCronJobRequest,
    ) -> CronResult<CronJobConfig> {
        // Normalize + validate schedule if provided
        let normalized_schedule = match &request.schedule {
            Some(schedule) => Some(Self::normalize_schedule(schedule)?),
            None => None,
        };

        let mut store = self.load().await?;

        let job = store
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .ok_or_else(|| CronServiceError::NotFound(id.to_string()))?;

        // Apply updates
        if let Some(name) = request.name {
            job.name = name;
        }
        if let Some(schedule) = normalized_schedule {
            job.schedule = schedule;
            job.next_run = None; // Will be recalculated by scheduler
        }
        if let Some(agent_id) = request.agent_id {
            job.agent_id = agent_id;
        }
        if let Some(message) = request.message {
            job.message = message;
        }
        if let Some(respond_to) = request.respond_to {
            job.respond_to = respond_to;
        }
        if let Some(enabled) = request.enabled {
            job.enabled = enabled;
        }
        if let Some(timezone) = request.timezone {
            job.timezone = Some(timezone);
        }
        if let Some(metadata) = request.metadata {
            job.metadata = Some(metadata);
        }
        job.updated_at = Some(chrono::Utc::now());

        let updated = job.clone();
        self.save(&store).await?;

        info!(job_id = %id, "Updated cron job");
        Ok(updated)
    }

    /// Delete a cron job.
    pub async fn delete(&self, id: &str) -> CronResult<()> {
        let mut store = self.load().await?;

        let initial_len = store.jobs.len();
        store.jobs.retain(|j| j.id != id);

        if store.jobs.len() == initial_len {
            return Err(CronServiceError::NotFound(id.to_string()));
        }

        self.save(&store).await?;
        info!(job_id = %id, "Deleted cron job");
        Ok(())
    }

    /// Enable a cron job.
    pub async fn enable(&self, id: &str) -> CronResult<CronJobConfig> {
        self.update(
            id,
            UpdateCronJobRequest {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await
    }

    /// Disable a cron job.
    pub async fn disable(&self, id: &str) -> CronResult<CronJobConfig> {
        self.update(
            id,
            UpdateCronJobRequest {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
    }

    /// Update last run time for a job.
    pub async fn update_last_run(&self, id: &str) -> CronResult<()> {
        let mut store = self.load().await?;

        if let Some(job) = store.jobs.iter_mut().find(|j| j.id == id) {
            job.last_run = Some(chrono::Utc::now());
            self.save(&store).await?;
        }

        Ok(())
    }

    /// Update next run time for a job.
    pub async fn update_next_run(
        &self,
        id: &str,
        next_run: chrono::DateTime<chrono::Utc>,
    ) -> CronResult<()> {
        let mut store = self.load().await?;

        if let Some(job) = store.jobs.iter_mut().find(|j| j.id == id) {
            job.next_run = Some(next_run);
            self.save(&store).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_primitives::vault_paths::VaultPaths;
    use std::sync::Arc;
    use tempfile::TempDir;

    async fn test_service() -> (CronService, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let paths = Arc::new(VaultPaths::new(temp_dir.path().to_path_buf()));
        let service = CronService::new(paths);
        (service, temp_dir)
    }

    #[tokio::test]
    async fn test_crud_operations() {
        let (service, _temp) = test_service().await;

        // Create
        let created = service
            .create(CreateCronJobRequest {
                id: "test-job".to_string(),
                name: "Test Job".to_string(),
                schedule: "0 0 * * * *".to_string(), // Every hour (sec min hour day month weekday)
                agent_id: "root".to_string(),
                message: "Test message".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            })
            .await
            .unwrap();

        assert_eq!(created.id, "test-job");
        assert!(created.created_at.is_some());

        // List
        let list = service.list().await.unwrap();
        assert_eq!(list.len(), 1);

        // Get
        let retrieved = service.get("test-job").await.unwrap();
        assert_eq!(retrieved.name, "Test Job");

        // Update
        let updated = service
            .update(
                "test-job",
                UpdateCronJobRequest {
                    name: Some("Updated Name".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.name, "Updated Name");

        // Delete
        service.delete("test-job").await.unwrap();
        let list = service.list().await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn test_invalid_schedule() {
        let (service, _temp) = test_service().await;

        let result = service
            .create(CreateCronJobRequest {
                id: "invalid".to_string(),
                name: "Invalid".to_string(),
                schedule: "invalid cron".to_string(),
                agent_id: "root".to_string(),
                message: "Test".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            })
            .await;

        assert!(matches!(result, Err(CronServiceError::InvalidSchedule(_))));
    }

    #[tokio::test]
    async fn test_duplicate_prevention() {
        let (service, _temp) = test_service().await;

        service
            .create(CreateCronJobRequest {
                id: "unique".to_string(),
                name: "First".to_string(),
                schedule: "0 0 * * * *".to_string(), // sec min hour day month weekday
                agent_id: "root".to_string(),
                message: "Test".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            })
            .await
            .unwrap();

        let result = service
            .create(CreateCronJobRequest {
                id: "unique".to_string(),
                name: "Second".to_string(),
                schedule: "0 0 * * * *".to_string(), // sec min hour day month weekday
                agent_id: "root".to_string(),
                message: "Test 2".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            })
            .await;

        assert!(matches!(result, Err(CronServiceError::AlreadyExists(_))));
    }

    #[test]
    fn normalize_schedule_passes_six_field_through() {
        let normalized = CronService::normalize_schedule("0 0 */4 * * *").unwrap();
        assert_eq!(normalized, "0 0 */4 * * *");
    }

    #[test]
    fn normalize_schedule_promotes_five_field_to_six() {
        // 5-field "every 5 minutes" → at second 0, every 5th minute
        let normalized = CronService::normalize_schedule("*/5 * * * *").unwrap();
        assert_eq!(normalized, "0 */5 * * * *");
    }

    #[test]
    fn normalize_schedule_trims_whitespace() {
        let normalized = CronService::normalize_schedule("  */15 * * * *  ").unwrap();
        assert_eq!(normalized, "0 */15 * * * *");
    }

    #[test]
    fn normalize_schedule_rejects_garbage() {
        assert!(matches!(
            CronService::normalize_schedule("nonsense"),
            Err(CronServiceError::InvalidSchedule(_))
        ));
    }

    #[tokio::test]
    async fn create_persists_normalized_schedule_for_five_field_input() {
        let (service, _temp) = test_service().await;

        let created = service
            .create(CreateCronJobRequest {
                id: "ui-style".to_string(),
                name: "UI-style".to_string(),
                schedule: "0 */6 * * *".to_string(), // 5-field, what the UI presets emit
                agent_id: "general-purpose".to_string(),
                message: "Test".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            })
            .await
            .unwrap();

        assert_eq!(created.schedule, "0 0 */6 * * *");
    }
}
