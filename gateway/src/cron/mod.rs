//! # Cron Scheduler
//!
//! Built-in scheduler for triggering agents on a schedule.
//!
//! The cron scheduler allows you to:
//! - Schedule agent executions at specific times (cron syntax)
//! - Route responses to connectors via `respond_to`
//! - Manage jobs through REST API
//!
//! ## Example
//!
//! ```ignore
//! // Create a daily report job
//! let job = CronJobConfig {
//!     id: "daily-report".to_string(),
//!     name: "Daily Report Generator".to_string(),
//!     schedule: "0 9 * * *".to_string(),  // 9am daily
//!     agent_id: "report-agent".to_string(),
//!     message: "Generate the daily sales report".to_string(),
//!     respond_to: vec!["email-connector".to_string()],
//!     enabled: true,
//!     timezone: Some("America/New_York".to_string()),
//!     ..Default::default()
//! };
//! ```

// Re-export config and service types from gateway-cron crate
pub use gateway_cron::*;

use crate::bus::{GatewayBus, SessionRequest};
use execution_state::TriggerSource;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio_cron_scheduler::{Job, JobScheduler, JobSchedulerError};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Default agent for cron jobs that don't specify (or leave blank) an
/// `agent_id`. Routes through the orchestrator, which can then delegate.
const DEFAULT_CRON_AGENT_ID: &str = "root";

/// Resolve which agent a cron job dispatches to. Trims the configured
/// `agent_id`; falls back to [`DEFAULT_CRON_AGENT_ID`] when empty.
fn resolve_cron_agent_id(configured: &str) -> &str {
    let trimmed = configured.trim();
    if trimmed.is_empty() {
        DEFAULT_CRON_AGENT_ID
    } else {
        trimmed
    }
}

/// Execute a `kind = "http"` cron action. Builds a `reqwest::Client`
/// per-call (cheap; cron firings are infrequent) so there's no shared
/// state to worry about. Errors are logged but never bubble — a cron
/// firing that fails does not block subsequent ticks.
async fn fire_http_action(job_id: &str, http: &gateway_cron::CronHttpAction) {
    info!(
        job_id = %job_id,
        method = %http.method,
        url = %http.url,
        "Cron job triggered (http)"
    );
    let method = match reqwest::Method::from_bytes(http.method.as_bytes()) {
        Ok(m) => m,
        Err(_) => {
            error!(
                job_id = %job_id,
                method = %http.method,
                "Cron http: invalid HTTP method"
            );
            return;
        }
    };
    let client = reqwest::Client::new();
    let mut req = client.request(method, &http.url);
    if let Some(body) = &http.body {
        req = req.json(body);
    }
    match req.send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                info!(
                    job_id = %job_id,
                    status = %status,
                    "Cron http succeeded"
                );
            } else {
                let body = resp.text().await.unwrap_or_default();
                warn!(
                    job_id = %job_id,
                    status = %status,
                    body = %body.chars().take(500).collect::<String>(),
                    "Cron http returned non-success status"
                );
            }
        }
        Err(e) => {
            error!(
                job_id = %job_id,
                error = %e,
                "Cron http request failed"
            );
        }
    }
}

/// The cron scheduler for scheduling agent executions.
pub struct CronScheduler {
    service: CronService,
    /// Wrapped in Mutex because shutdown() requires &mut self
    scheduler: Mutex<JobScheduler>,
    /// Maps job ID to scheduler UUID for management
    job_uuids: RwLock<HashMap<String, Uuid>>,
    /// Per-job cancel flag captured by the scheduled closure. Set on
    /// unschedule so a tick that fires while delete is in flight (or that
    /// was already queued by `tokio-cron-scheduler`'s 500ms tick loop)
    /// returns without submitting a new session.
    job_cancels: RwLock<HashMap<String, Arc<AtomicBool>>>,
    /// Gateway bus for submitting sessions
    bus: Arc<dyn GatewayBus>,
}

impl CronScheduler {
    /// Create a new cron scheduler.
    pub async fn new(
        service: CronService,
        bus: Arc<dyn GatewayBus>,
    ) -> Result<Self, CronSchedulerError> {
        let scheduler = JobScheduler::new().await?;

        Ok(Self {
            service,
            scheduler: Mutex::new(scheduler),
            job_uuids: RwLock::new(HashMap::new()),
            job_cancels: RwLock::new(HashMap::new()),
            bus,
        })
    }

    /// Initialize and start the scheduler.
    ///
    /// This loads all enabled jobs from disk and schedules them.
    pub async fn start(&self) -> Result<(), CronSchedulerError> {
        info!("Starting cron scheduler");

        // Load jobs from disk
        let jobs = self.service.load().await?;
        let enabled_count = jobs.jobs.iter().filter(|j| j.enabled).count();

        info!(
            total = jobs.jobs.len(),
            enabled = enabled_count,
            "Loading cron jobs"
        );

        // Schedule each enabled job
        for job_config in jobs.jobs.iter().filter(|j| j.enabled) {
            if let Err(e) = self.schedule_job(job_config).await {
                warn!(
                    job_id = %job_config.id,
                    error = %e,
                    "Failed to schedule job"
                );
            }
        }

        // Start the scheduler
        {
            let scheduler = self.scheduler.lock().await;
            scheduler.start().await?;
        }

        info!(enabled = enabled_count, "Cron scheduler started");
        Ok(())
    }

    /// Stop the scheduler.
    pub async fn stop(&self) -> Result<(), CronSchedulerError> {
        info!("Stopping cron scheduler");
        let mut scheduler = self.scheduler.lock().await;
        scheduler.shutdown().await?;
        Ok(())
    }

    /// Schedule a job.
    async fn schedule_job(&self, job_config: &CronJobConfig) -> Result<(), CronSchedulerError> {
        let job_id = job_config.id.clone();
        let kind = job_config.kind;
        let agent_id = job_config.agent_id.clone();
        let message = job_config.message.clone();
        let http = job_config.http.clone();
        let respond_to = job_config.respond_to.clone();
        let bus = self.bus.clone();
        let service = self.service.clone();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        self.job_cancels
            .write()
            .await
            .insert(job_id.clone(), cancel_flag.clone());

        debug!(
            job_id = %job_id,
            schedule = %job_config.schedule,
            kind = ?kind,
            "Scheduling cron job"
        );

        let job = Job::new_async(job_config.schedule.as_str(), move |_uuid, _lock| {
            let job_id = job_id.clone();
            let agent_id = agent_id.clone();
            let message = message.clone();
            let http = http.clone();
            let respond_to = respond_to.clone();
            let bus = bus.clone();
            let service = service.clone();
            let cancel_flag = cancel_flag.clone();

            Box::pin(async move {
                if cancel_flag.load(Ordering::Acquire) {
                    debug!(
                        job_id = %job_id,
                        "Cron tick fired but job is cancelled; skipping submission"
                    );
                    return;
                }

                match kind {
                    gateway_cron::CronJobKind::Agent => {
                        let target_agent = resolve_cron_agent_id(&agent_id).to_string();
                        info!(
                            job_id = %job_id,
                            agent_id = %target_agent,
                            "Cron job triggered (agent)"
                        );
                        let request = SessionRequest::new(&target_agent, &message)
                            .with_source(TriggerSource::Cron)
                            .with_external_ref(format!("cron-{}", job_id))
                            .with_respond_to(respond_to);
                        match bus.submit(request).await {
                            Ok(handle) => {
                                info!(
                                    job_id = %job_id,
                                    session_id = %handle.session_id,
                                    execution_id = %handle.execution_id,
                                    "Cron job submitted successfully"
                                );
                            }
                            Err(e) => {
                                error!(
                                    job_id = %job_id,
                                    error = %e,
                                    "Failed to submit cron job"
                                );
                            }
                        }
                    }
                    gateway_cron::CronJobKind::Http => {
                        let Some(http) = http.as_ref() else {
                            error!(
                                job_id = %job_id,
                                "Cron job marked kind=http but `http` payload is missing; skipping"
                            );
                            return;
                        };
                        fire_http_action(&job_id, http).await;
                    }
                }

                // Update last run time regardless of kind.
                if let Err(e) = service.update_last_run(&job_id).await {
                    warn!(
                        job_id = %job_id,
                        error = %e,
                        "Failed to update last run time"
                    );
                }
            })
        })?;

        let uuid = job.guid();
        {
            let scheduler = self.scheduler.lock().await;
            scheduler.add(job).await?;
        }

        // Store the UUID for later management
        self.job_uuids
            .write()
            .await
            .insert(job_config.id.clone(), uuid);

        info!(
            job_id = %job_config.id,
            schedule = %job_config.schedule,
            "Cron job scheduled"
        );

        Ok(())
    }

    /// Unschedule a job.
    ///
    /// Flips the per-job cancel flag first so any tick already queued by
    /// `tokio-cron-scheduler`'s 500ms tick loop short-circuits without
    /// submitting a session, then removes the job from the live scheduler
    /// and the UUID map. Logs a `warn` (not silent success) if the UUID is
    /// missing — that means the live scheduler may still hold the job and
    /// we want it visible in logs rather than masked.
    async fn unschedule_job(&self, job_id: &str) -> Result<(), CronSchedulerError> {
        if let Some(flag) = self.job_cancels.write().await.remove(job_id) {
            flag.store(true, Ordering::Release);
        }

        let uuid = self.job_uuids.write().await.remove(job_id);

        match uuid {
            Some(uuid) => {
                let scheduler = self.scheduler.lock().await;
                scheduler.remove(&uuid).await?;
                info!(job_id = %job_id, "Cron job unscheduled");
            }
            None => {
                warn!(
                    job_id = %job_id,
                    "unschedule_job: no UUID found in job_uuids; live scheduler entry may be orphaned"
                );
            }
        }

        Ok(())
    }

    /// Reschedule a job (unschedule and schedule again).
    pub async fn reschedule_job(&self, job_id: &str) -> Result<(), CronSchedulerError> {
        self.unschedule_job(job_id).await?;

        let job = self.service.get(job_id).await?;
        if job.enabled {
            self.schedule_job(&job).await?;
        }

        Ok(())
    }

    /// Get the service for CRUD operations.
    pub fn service(&self) -> &CronService {
        &self.service
    }

    /// Manually trigger a job.
    pub async fn trigger(&self, job_id: &str) -> Result<TriggerResult, CronSchedulerError> {
        let job = self.service.get(job_id).await?;
        let target_agent = resolve_cron_agent_id(&job.agent_id).to_string();

        info!(
            job_id = %job_id,
            agent_id = %target_agent,
            "Manually triggering cron job"
        );

        let request = SessionRequest::new(&target_agent, &job.message)
            .with_source(TriggerSource::Cron)
            .with_external_ref(format!("cron-{}-manual", job_id))
            .with_respond_to(job.respond_to.clone());

        // Submit to gateway bus
        match self.bus.submit(request).await {
            Ok(handle) => {
                // Update last run time
                let _ = self.service.update_last_run(job_id).await;

                Ok(TriggerResult {
                    success: true,
                    session_id: Some(handle.session_id),
                    execution_id: Some(handle.execution_id),
                    message: "Job triggered successfully".to_string(),
                })
            }
            Err(e) => Ok(TriggerResult {
                success: false,
                session_id: None,
                execution_id: None,
                message: format!("Failed to trigger job: {}", e),
            }),
        }
    }

    /// Create a new job and schedule it if enabled.
    pub async fn create_job(&self, request: CreateCronJobRequest) -> CronResult<CronJobConfig> {
        let job = self.service.create(request).await?;

        if job.enabled {
            if let Err(e) = self.schedule_job(&job).await {
                warn!(
                    job_id = %job.id,
                    error = %e,
                    "Job created but failed to schedule"
                );
            }
        }

        Ok(job)
    }

    /// Update a job and reschedule if needed.
    pub async fn update_job(
        &self,
        id: &str,
        request: UpdateCronJobRequest,
    ) -> CronResult<CronJobConfig> {
        let schedule_changed = request.schedule.is_some();
        let enabled_changed = request.enabled.is_some();

        let job = self.service.update(id, request).await?;

        // Reschedule if schedule or enabled status changed
        if schedule_changed || enabled_changed {
            if let Err(e) = self.reschedule_job(id).await {
                warn!(
                    job_id = %id,
                    error = %e,
                    "Job updated but failed to reschedule"
                );
            }
        }

        Ok(job)
    }

    /// Delete a job and unschedule it.
    pub async fn delete_job(&self, id: &str) -> CronResult<()> {
        // Unschedule first
        if let Err(e) = self.unschedule_job(id).await {
            warn!(
                job_id = %id,
                error = %e,
                "Failed to unschedule job before deletion"
            );
        }

        self.service.delete(id).await
    }

    /// Enable a job and schedule it.
    pub async fn enable_job(&self, id: &str) -> CronResult<CronJobConfig> {
        let job = self.service.enable(id).await?;

        if let Err(e) = self.schedule_job(&job).await {
            warn!(
                job_id = %id,
                error = %e,
                "Job enabled but failed to schedule"
            );
        }

        Ok(job)
    }

    /// Disable a job and unschedule it.
    pub async fn disable_job(&self, id: &str) -> CronResult<CronJobConfig> {
        if let Err(e) = self.unschedule_job(id).await {
            warn!(
                job_id = %id,
                error = %e,
                "Failed to unschedule job"
            );
        }

        self.service.disable(id).await
    }

    /// List all jobs.
    pub async fn list_jobs(&self) -> CronResult<Vec<CronJobConfig>> {
        self.service.list().await
    }

    /// Get a job by ID.
    pub async fn get_job(&self, id: &str) -> CronResult<CronJobConfig> {
        self.service.get(id).await
    }
}

/// Errors from the cron scheduler.
#[derive(Debug, thiserror::Error)]
pub enum CronSchedulerError {
    #[error("Service error: {0}")]
    Service(#[from] CronServiceError),

    #[error("Scheduler error: {0}")]
    Scheduler(#[from] JobSchedulerError),
}

#[cfg(test)]
mod tests {
    use super::{resolve_cron_agent_id, DEFAULT_CRON_AGENT_ID};

    #[test]
    fn returns_configured_agent_id_when_present() {
        assert_eq!(resolve_cron_agent_id("general-purpose"), "general-purpose");
        assert_eq!(resolve_cron_agent_id("research-agent"), "research-agent");
    }

    #[test]
    fn falls_back_to_root_when_empty() {
        assert_eq!(resolve_cron_agent_id(""), DEFAULT_CRON_AGENT_ID);
    }

    #[test]
    fn falls_back_to_root_when_whitespace_only() {
        assert_eq!(resolve_cron_agent_id("   "), DEFAULT_CRON_AGENT_ID);
        assert_eq!(resolve_cron_agent_id("\t\n"), DEFAULT_CRON_AGENT_ID);
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(resolve_cron_agent_id("  builder-agent  "), "builder-agent");
    }

    #[test]
    fn passes_through_explicit_root() {
        assert_eq!(resolve_cron_agent_id("root"), "root");
    }

    #[test]
    fn bundled_default_cron_template_parses() {
        let bytes = gateway_templates::Templates::get("default_cron.json")
            .expect("default_cron.json bundled in gateway-templates")
            .data;

        let requests: Vec<gateway_cron::CreateCronJobRequest> = serde_json::from_slice(&bytes)
            .expect("default_cron.json must match CreateCronJobRequest schema");

        let cleanup = requests
            .iter()
            .find(|r| r.id == "default-cleanup")
            .expect("bundled `default-cleanup` cron job missing");

        assert_eq!(cleanup.agent_id, "general-purpose");
        assert_eq!(cleanup.schedule, "0 0 */4 * * *");
        assert!(cleanup.enabled);
    }

    mod scheduler {
        use super::super::*;
        use crate::bus::{GatewayBus, SessionRequest};
        use agent_primitives::vault_paths::VaultPaths;
        use async_trait::async_trait;
        use execution_state::SessionStatus;
        use gateway_bus::{BusError, SessionHandle};
        use gateway_cron::{CreateCronJobRequest, CronJobConfig, CronService};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use tempfile::TempDir;

        /// Test bus that records `submit` calls without doing real work.
        struct CountingBus {
            calls: AtomicUsize,
        }

        impl CountingBus {
            fn new() -> Arc<Self> {
                Arc::new(Self {
                    calls: AtomicUsize::new(0),
                })
            }
        }

        #[async_trait]
        impl GatewayBus for CountingBus {
            async fn submit(&self, _request: SessionRequest) -> Result<SessionHandle, BusError> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(SessionHandle::new(
                    "sess-test".to_string(),
                    "exec-test".to_string(),
                    "conv-test".to_string(),
                ))
            }
            async fn status(&self, _session_id: &str) -> Result<SessionStatus, BusError> {
                Err(BusError::SessionNotFound("test".to_string()))
            }
            async fn cancel(&self, _session_id: &str) -> Result<(), BusError> {
                Ok(())
            }
            async fn pause(&self, _session_id: &str) -> Result<(), BusError> {
                Ok(())
            }
            async fn resume(&self, _session_id: &str) -> Result<(), BusError> {
                Ok(())
            }
        }

        async fn build_scheduler() -> (CronScheduler, Arc<CountingBus>, TempDir) {
            let temp = TempDir::new().unwrap();
            let paths = Arc::new(VaultPaths::new(temp.path().to_path_buf()));
            let service = CronService::new(paths);
            let bus = CountingBus::new();
            let scheduler = CronScheduler::new(service, bus.clone()).await.unwrap();
            (scheduler, bus, temp)
        }

        fn sample_config(id: &str) -> CronJobConfig {
            CronJobConfig {
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
                last_run: None,
                next_run: None,
                created_at: None,
                updated_at: None,
            }
        }

        #[tokio::test]
        async fn schedule_populates_uuid_and_cancel_maps() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            scheduler.schedule_job(&sample_config("j1")).await.unwrap();

            assert!(scheduler.job_uuids.read().await.contains_key("j1"));
            assert!(scheduler.job_cancels.read().await.contains_key("j1"));
        }

        #[tokio::test]
        async fn unschedule_sets_cancel_flag_and_clears_maps() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            scheduler.schedule_job(&sample_config("j1")).await.unwrap();

            // Snapshot the cancel flag before unscheduling so we can verify
            // it was flipped (the entry itself is removed from the map).
            let cancel_arc = scheduler
                .job_cancels
                .read()
                .await
                .get("j1")
                .cloned()
                .expect("cancel flag present after schedule");
            assert!(!cancel_arc.load(Ordering::Acquire));

            scheduler.unschedule_job("j1").await.unwrap();

            assert!(
                cancel_arc.load(Ordering::Acquire),
                "cancel flag must be set so a queued tick short-circuits"
            );
            assert!(!scheduler.job_uuids.read().await.contains_key("j1"));
            assert!(!scheduler.job_cancels.read().await.contains_key("j1"));
        }

        #[tokio::test]
        async fn unschedule_unknown_id_is_ok() {
            // Returns Ok even when the id was never scheduled — but emits a
            // warn (covered by tracing tests in the wider workspace, not
            // here). The caller's invariant is just "did delete succeed".
            let (scheduler, _bus, _temp) = build_scheduler().await;
            scheduler.unschedule_job("missing").await.unwrap();
        }

        #[tokio::test]
        async fn delete_job_unschedules_then_removes_from_disk() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            let request = CreateCronJobRequest {
                id: "j1".to_string(),
                name: "j1".to_string(),
                schedule: "0 0 */4 * * *".to_string(),
                agent_id: "general-purpose".to_string(),
                message: "noop".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            };
            let job = scheduler.create_job(request).await.unwrap();
            assert_eq!(job.id, "j1");

            let cancel_arc = scheduler
                .job_cancels
                .read()
                .await
                .get("j1")
                .cloned()
                .expect("cancel flag set on create");

            scheduler.delete_job("j1").await.unwrap();

            assert!(cancel_arc.load(Ordering::Acquire));
            assert!(!scheduler.job_uuids.read().await.contains_key("j1"));
            assert!(!scheduler.job_cancels.read().await.contains_key("j1"));
            assert!(scheduler.get_job("j1").await.is_err());
        }

        #[tokio::test]
        async fn create_job_returns_disabled_without_scheduling() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            let request = CreateCronJobRequest {
                id: "j-disabled".to_string(),
                name: "j-disabled".to_string(),
                schedule: "0 0 */4 * * *".to_string(),
                agent_id: "general-purpose".to_string(),
                message: "noop".to_string(),
                respond_to: vec![],
                enabled: false,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            };
            let job = scheduler.create_job(request).await.unwrap();
            assert!(!job.enabled);
            assert!(!scheduler.job_uuids.read().await.contains_key("j-disabled"));
            assert!(!scheduler
                .job_cancels
                .read()
                .await
                .contains_key("j-disabled"));
        }

        #[tokio::test]
        async fn enable_job_then_disable_job_round_trips() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            let request = CreateCronJobRequest {
                id: "j2".to_string(),
                name: "j2".to_string(),
                schedule: "0 0 */4 * * *".to_string(),
                agent_id: "general-purpose".to_string(),
                message: "noop".to_string(),
                respond_to: vec![],
                enabled: false,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            };
            scheduler.create_job(request).await.unwrap();
            assert!(!scheduler.job_uuids.read().await.contains_key("j2"));

            let job = scheduler.enable_job("j2").await.unwrap();
            assert!(job.enabled);
            assert!(scheduler.job_uuids.read().await.contains_key("j2"));

            let job = scheduler.disable_job("j2").await.unwrap();
            assert!(!job.enabled);
            assert!(!scheduler.job_uuids.read().await.contains_key("j2"));
        }

        #[tokio::test]
        async fn list_jobs_returns_seeded_jobs() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            assert!(scheduler.list_jobs().await.unwrap().is_empty());

            let request = CreateCronJobRequest {
                id: "alpha".to_string(),
                name: "alpha".to_string(),
                schedule: "0 0 */4 * * *".to_string(),
                agent_id: "general-purpose".to_string(),
                message: "noop".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            };
            scheduler.create_job(request).await.unwrap();
            let jobs = scheduler.list_jobs().await.unwrap();
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].id, "alpha");
        }

        #[tokio::test]
        async fn get_job_returns_existing_record() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            let request = CreateCronJobRequest {
                id: "beta".to_string(),
                name: "beta-display".to_string(),
                schedule: "0 0 */4 * * *".to_string(),
                agent_id: "general-purpose".to_string(),
                message: "noop".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            };
            scheduler.create_job(request).await.unwrap();
            let job = scheduler.get_job("beta").await.unwrap();
            assert_eq!(job.name, "beta-display");
        }

        #[tokio::test]
        async fn get_job_returns_err_for_missing_id() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            assert!(scheduler.get_job("nope").await.is_err());
        }

        #[tokio::test]
        async fn trigger_submits_to_bus_and_returns_handle() {
            let (scheduler, bus, _temp) = build_scheduler().await;
            let request = CreateCronJobRequest {
                id: "j-trig".to_string(),
                name: "trig".to_string(),
                schedule: "0 0 */4 * * *".to_string(),
                agent_id: "general-purpose".to_string(),
                message: "ping".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            };
            scheduler.create_job(request).await.unwrap();

            let result = scheduler.trigger("j-trig").await.unwrap();
            assert!(result.success);
            assert_eq!(result.session_id.as_deref(), Some("sess-test"));
            assert_eq!(result.execution_id.as_deref(), Some("exec-test"));
            assert_eq!(bus.calls.load(Ordering::SeqCst), 1);
        }

        #[tokio::test]
        async fn trigger_uses_root_when_agent_id_blank() {
            let (scheduler, bus, _temp) = build_scheduler().await;
            let request = CreateCronJobRequest {
                id: "j-blank".to_string(),
                name: "j-blank".to_string(),
                schedule: "0 0 */4 * * *".to_string(),
                agent_id: "   ".to_string(),
                message: "noop".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            };
            scheduler.create_job(request).await.unwrap();
            let result = scheduler.trigger("j-blank").await.unwrap();
            assert!(result.success);
            assert_eq!(bus.calls.load(Ordering::SeqCst), 1);
        }

        #[tokio::test]
        async fn trigger_returns_err_for_missing_id() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            assert!(scheduler.trigger("ghost").await.is_err());
        }

        struct FailingBus;
        #[async_trait]
        impl GatewayBus for FailingBus {
            async fn submit(&self, _r: SessionRequest) -> Result<SessionHandle, BusError> {
                Err(BusError::Internal("rate-limited".to_string()))
            }
            async fn status(&self, _id: &str) -> Result<SessionStatus, BusError> {
                Err(BusError::SessionNotFound("x".into()))
            }
            async fn cancel(&self, _id: &str) -> Result<(), BusError> {
                Ok(())
            }
            async fn pause(&self, _id: &str) -> Result<(), BusError> {
                Ok(())
            }
            async fn resume(&self, _id: &str) -> Result<(), BusError> {
                Ok(())
            }
        }

        #[tokio::test]
        async fn trigger_reports_failure_when_bus_errors() {
            let temp = TempDir::new().unwrap();
            let paths = Arc::new(VaultPaths::new(temp.path().to_path_buf()));
            let service = CronService::new(paths);
            let bus: Arc<dyn GatewayBus> = Arc::new(FailingBus);
            let scheduler = CronScheduler::new(service, bus).await.unwrap();
            let request = CreateCronJobRequest {
                id: "j".to_string(),
                name: "j".to_string(),
                schedule: "0 0 */4 * * *".to_string(),
                agent_id: "general-purpose".to_string(),
                message: "noop".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            };
            scheduler.create_job(request).await.unwrap();

            let result = scheduler.trigger("j").await.unwrap();
            assert!(!result.success);
            assert!(result.session_id.is_none());
            assert!(result.message.contains("rate-limited"));
        }

        #[tokio::test]
        async fn update_job_reschedules_when_schedule_changes() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            let request = CreateCronJobRequest {
                id: "j".to_string(),
                name: "j".to_string(),
                schedule: "0 0 */4 * * *".to_string(),
                agent_id: "general-purpose".to_string(),
                message: "noop".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            };
            scheduler.create_job(request).await.unwrap();
            let original = scheduler
                .job_cancels
                .read()
                .await
                .get("j")
                .cloned()
                .unwrap();

            let update = UpdateCronJobRequest {
                schedule: Some("0 0 */6 * * *".to_string()),
                ..Default::default()
            };
            scheduler.update_job("j", update).await.unwrap();

            assert!(original.load(Ordering::Acquire));
            let replacement = scheduler
                .job_cancels
                .read()
                .await
                .get("j")
                .cloned()
                .unwrap();
            assert!(!replacement.load(Ordering::Acquire));
        }

        #[tokio::test]
        async fn update_job_leaves_schedule_when_only_metadata_changes() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            let request = CreateCronJobRequest {
                id: "j".to_string(),
                name: "old name".to_string(),
                schedule: "0 0 */4 * * *".to_string(),
                agent_id: "general-purpose".to_string(),
                message: "noop".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            };
            scheduler.create_job(request).await.unwrap();
            let original = scheduler
                .job_cancels
                .read()
                .await
                .get("j")
                .cloned()
                .unwrap();

            let update = UpdateCronJobRequest {
                name: Some("new name".to_string()),
                ..Default::default()
            };
            let job = scheduler.update_job("j", update).await.unwrap();
            assert_eq!(job.name, "new name");

            let same = scheduler
                .job_cancels
                .read()
                .await
                .get("j")
                .cloned()
                .unwrap();
            assert!(Arc::ptr_eq(&original, &same));
            assert!(!same.load(Ordering::Acquire));
        }

        #[tokio::test]
        async fn delete_job_for_missing_id_returns_err() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            assert!(scheduler.delete_job("ghost").await.is_err());
        }

        #[tokio::test]
        async fn cron_scheduler_error_display_carries_inner_message() {
            let svc_err = CronSchedulerError::Service(CronServiceError::NotFound("x".into()));
            assert!(svc_err.to_string().contains("Service error"));
        }

        #[tokio::test]
        async fn service_accessor_returns_underlying_handle() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            let svc = scheduler.service();
            assert!(svc.list().await.unwrap().is_empty());
        }

        #[tokio::test]
        async fn start_then_stop_runs_lifecycle() {
            let (scheduler, _bus, _temp) = build_scheduler().await;
            scheduler.start().await.unwrap();
            scheduler.stop().await.unwrap();
        }

        #[tokio::test]
        async fn reschedule_replaces_cancel_flag() {
            // After update→reschedule, the closure for the old schedule
            // must see a cancelled flag (so any in-flight tick is dropped),
            // and the new closure must hold a fresh, un-cancelled flag.
            let (scheduler, _bus, _temp) = build_scheduler().await;
            let request = CreateCronJobRequest {
                id: "j1".to_string(),
                name: "j1".to_string(),
                schedule: "0 0 */4 * * *".to_string(),
                agent_id: "general-purpose".to_string(),
                message: "noop".to_string(),
                respond_to: vec![],
                enabled: true,
                timezone: None,
                metadata: None,
                kind: Default::default(),
                http: None,
            };
            scheduler.create_job(request).await.unwrap();
            let original = scheduler
                .job_cancels
                .read()
                .await
                .get("j1")
                .cloned()
                .unwrap();

            scheduler.reschedule_job("j1").await.unwrap();

            let replacement = scheduler
                .job_cancels
                .read()
                .await
                .get("j1")
                .cloned()
                .unwrap();
            assert!(
                original.load(Ordering::Acquire),
                "old cancel flag must fire so queued ticks abort"
            );
            assert!(
                !replacement.load(Ordering::Acquire),
                "new cancel flag must start un-cancelled"
            );
            assert!(!Arc::ptr_eq(&original, &replacement));
        }
    }
}
