//! HTTP Gateway Bus implementation.
//!
//! This implementation wraps the [`ExecutionRunner`] to provide the [`GatewayBus`]
//! interface for HTTP-based session management.

use super::{BusError, GatewayBus, SessionHandle, SessionRequest};
use crate::execution::{ExecutionConfig, ExecutionRunner};
use async_trait::async_trait;
use execution_state::{SessionStatus, StateService};
use std::path::PathBuf;
use std::sync::Arc;
use zbot_stores_sqlite::DatabaseManager;

/// HTTP-based Gateway Bus implementation.
///
/// This is the default implementation that wraps the existing [`ExecutionRunner`]
/// to provide a unified interface for session management.
///
/// # Example
///
/// ```ignore
/// use gateway::bus::{HttpGatewayBus, GatewayBus, SessionRequest};
///
/// let bus = HttpGatewayBus::new(runner, state_service, config_dir);
///
/// let request = SessionRequest::new("root", "Hello!")
///     .with_source(TriggerSource::Web);
///
/// let handle = bus.submit(request).await?;
/// ```
pub struct HttpGatewayBus {
    /// The underlying execution runner.
    runner: Arc<ExecutionRunner>,
    /// State service for session/execution management.
    state_service: Arc<StateService<DatabaseManager>>,
    /// Configuration directory (vault path).
    config_dir: PathBuf,
}

impl HttpGatewayBus {
    /// Create a new HTTP Gateway Bus.
    ///
    /// # Arguments
    ///
    /// * `runner` - The execution runner to use for agent invocations
    /// * `state_service` - The state service for session/execution queries
    /// * `config_dir` - The configuration directory (vault path)
    pub fn new(
        runner: Arc<ExecutionRunner>,
        state_service: Arc<StateService<DatabaseManager>>,
        config_dir: PathBuf,
    ) -> Self {
        Self {
            runner,
            state_service,
            config_dir,
        }
    }

    /// Get a reference to the underlying execution runner.
    ///
    /// This is useful for operations that need direct access to the runner,
    /// such as getting execution handles.
    pub fn runner(&self) -> &Arc<ExecutionRunner> {
        &self.runner
    }

    /// Get a reference to the state service.
    pub fn state_service(&self) -> &Arc<StateService<DatabaseManager>> {
        &self.state_service
    }

    /// Convert a SessionRequest into an ExecutionConfig.
    fn to_execution_config(&self, request: &SessionRequest) -> ExecutionConfig {
        // Generate conversation ID if not provided
        let conversation_id = request
            .conversation_id
            .clone()
            .unwrap_or_else(|| format!("web-{}", uuid::Uuid::new_v4()));

        let mut config = ExecutionConfig::new(
            request.agent_id.clone(),
            conversation_id,
            self.config_dir.clone(),
        )
        .with_source(request.source);

        // Set session ID if continuing an existing session
        if let Some(session_id) = &request.session_id {
            config = config.with_session_id(session_id.clone());
        }

        // Set connector IDs for response routing
        if let Some(respond_to) = &request.respond_to {
            if !respond_to.is_empty() {
                config = config.with_respond_to(respond_to.clone());
            }
        }

        // Pass through routing fields for connector dispatch
        if let Some(thread_id) = &request.thread_id {
            config = config.with_thread_id(thread_id.clone());
        }
        if let Some(connector_id) = &request.connector_id {
            config = config.with_connector_id(connector_id.clone());
        }

        // Copy metadata from request
        if let Some(metadata) = &request.metadata {
            config = config.with_metadata(metadata.clone());
        }

        config
    }
}

#[async_trait]
impl GatewayBus for HttpGatewayBus {
    async fn submit(&self, mut request: SessionRequest) -> Result<SessionHandle, BusError> {
        // If thread_id provided but no session_id, try to find existing session
        if request.thread_id.is_some() && request.session_id.is_none() {
            if let Some(thread_id) = &request.thread_id {
                if let Ok(Some(existing)) = self.state_service.find_session_by_thread_id(thread_id)
                {
                    tracing::info!(
                        thread_id = %thread_id,
                        session_id = %existing.id,
                        "Continuing existing session for thread"
                    );
                    request = request.with_session_id(existing.id);
                }
            }
        }

        let config = self.to_execution_config(&request);
        let conversation_id = config.conversation_id.clone();

        // Invoke the runner
        let (_handle, session_id) = self
            .runner
            .invoke(config, request.message.clone())
            .await
            .map_err(BusError::Internal)?;

        // Get the most recent execution for this session to retrieve its ID
        let execution_id = self
            .state_service
            .get_session_with_executions(&session_id)
            .ok()
            .flatten()
            .and_then(|swe| {
                swe.executions
                    .into_iter()
                    .max_by_key(|e| e.started_at.clone())
                    .map(|e| e.id)
            })
            .unwrap_or_else(|| format!("exec-{}", uuid::Uuid::new_v4()));

        Ok(SessionHandle::new(
            session_id,
            execution_id,
            conversation_id,
        ))
    }

    async fn status(&self, session_id: &str) -> Result<SessionStatus, BusError> {
        self.state_service
            .get_session(session_id)
            .map_err(BusError::Internal)?
            .map(|s| s.status)
            .ok_or_else(|| BusError::SessionNotFound(session_id.to_string()))
    }

    async fn cancel(&self, session_id: &str) -> Result<(), BusError> {
        self.runner
            .cancel(session_id)
            .await
            .map_err(BusError::Internal)
    }

    async fn pause(&self, session_id: &str) -> Result<(), BusError> {
        self.runner
            .pause(session_id)
            .await
            .map_err(BusError::Internal)
    }

    async fn resume(&self, session_id: &str) -> Result<(), BusError> {
        self.runner
            .resume(session_id)
            .await
            .map_err(BusError::Internal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use execution_state::TriggerSource;

    // Note: Full integration tests would require setting up the ExecutionRunner
    // which has many dependencies. These unit tests focus on the request/response
    // data structures and conversion logic.

    // ==================== SessionRequest Conversion Tests ====================

    #[test]
    fn session_request_with_explicit_conversation_id() {
        let request = SessionRequest::new("root", "Hello")
            .with_session_id("sess-123")
            .with_conversation_id("conv-456");

        assert_eq!(request.agent_id, "root");
        assert_eq!(request.message, "Hello");
        assert_eq!(request.session_id, Some("sess-123".to_string()));
        assert_eq!(request.conversation_id, Some("conv-456".to_string()));
    }

    #[test]
    fn session_request_without_conversation_id() {
        let request = SessionRequest::new("root", "Hello");

        assert_eq!(request.agent_id, "root");
        assert_eq!(request.message, "Hello");
        assert!(request.conversation_id.is_none());
        // When None, http_bus should generate one like "web-{uuid}"
    }

    #[test]
    fn session_request_new_session_vs_continue() {
        // New session: no session_id
        let new_session = SessionRequest::new("root", "Start conversation");
        assert!(new_session.session_id.is_none());

        // Continue session: has session_id
        let continue_session =
            SessionRequest::new("root", "Continue conversation").with_session_id("sess-existing");
        assert!(continue_session.session_id.is_some());
    }

    #[test]
    fn session_request_all_trigger_sources() {
        let sources = [
            (TriggerSource::Web, "web"),
            (TriggerSource::Cli, "cli"),
            (TriggerSource::Cron, "cron"),
            (TriggerSource::Api, "api"),
            (TriggerSource::Connector, "connector"),
        ];

        for (source, name) in sources {
            let request = SessionRequest::new("root", "test").with_source(source);
            assert_eq!(request.source, source, "Source mismatch for {}", name);
        }
    }

    #[test]
    fn session_request_priority_ordering() {
        // Lower priority number = higher priority
        let high_priority = SessionRequest::new("root", "urgent").with_priority(1);
        let medium_priority = SessionRequest::new("root", "normal").with_priority(50);
        let low_priority = SessionRequest::new("root", "background").with_priority(100);

        assert!(high_priority.priority.unwrap() < medium_priority.priority.unwrap());
        assert!(medium_priority.priority.unwrap() < low_priority.priority.unwrap());
    }

    #[test]
    fn session_request_external_ref_for_correlation() {
        // External ref is used to correlate with external systems
        let webhook_request = SessionRequest::new("handler", "Process webhook")
            .with_source(TriggerSource::Api)
            .with_external_ref("github-webhook-event-12345");

        assert_eq!(
            webhook_request.external_ref,
            Some("github-webhook-event-12345".to_string())
        );

        let email_request = SessionRequest::new("email-agent", "Process email")
            .with_source(TriggerSource::Connector)
            .with_external_ref("email-msg-id-abc123");

        assert_eq!(
            email_request.external_ref,
            Some("email-msg-id-abc123".to_string())
        );
    }

    #[test]
    fn session_request_metadata_json() {
        let metadata = serde_json::json!({
            "user_id": "user-123",
            "organization": "acme-corp",
            "tags": ["important", "automated"],
            "nested": {
                "level": 2
            }
        });

        let request = SessionRequest::new("root", "test").with_metadata(metadata.clone());

        assert!(request.metadata.is_some());
        let meta = request.metadata.unwrap();
        assert_eq!(meta["user_id"], "user-123");
        assert_eq!(meta["tags"][0], "important");
        assert_eq!(meta["nested"]["level"], 2);
    }

    // ==================== SessionHandle Tests ====================

    #[test]
    fn session_handle_contains_all_ids() {
        let handle = SessionHandle::new("sess-abc", "exec-def", "conv-ghi");

        assert!(handle.session_id.starts_with("sess-"));
        assert!(handle.execution_id.starts_with("exec-"));
        assert!(handle.conversation_id.starts_with("conv-"));
    }

    #[test]
    fn session_handle_json_roundtrip() {
        let original = SessionHandle::new("sess-test", "exec-test", "conv-test");

        let json = serde_json::to_string(&original).unwrap();
        let parsed: SessionHandle = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.session_id, original.session_id);
        assert_eq!(parsed.execution_id, original.execution_id);
        assert_eq!(parsed.conversation_id, original.conversation_id);
    }

    // ==================== BusError Tests ====================

    #[test]
    fn bus_error_session_not_found() {
        let err = BusError::SessionNotFound("sess-nonexistent".to_string());

        match &err {
            BusError::SessionNotFound(id) => assert_eq!(id, "sess-nonexistent"),
            _ => panic!("Expected SessionNotFound"),
        }

        assert!(err.to_string().contains("sess-nonexistent"));
    }

    #[test]
    fn bus_error_execution_not_found() {
        let err = BusError::ExecutionNotFound("exec-nonexistent".to_string());

        match &err {
            BusError::ExecutionNotFound(id) => assert_eq!(id, "exec-nonexistent"),
            _ => panic!("Expected ExecutionNotFound"),
        }
    }

    #[test]
    fn bus_error_invalid_state_for_cancel() {
        // Can't cancel a completed session
        let err = BusError::InvalidState {
            session_id: "sess-123".to_string(),
            current_state: "completed".to_string(),
            expected_states: vec!["running".to_string(), "paused".to_string()],
        };

        let msg = err.to_string();
        assert!(msg.contains("sess-123"));
        assert!(msg.contains("completed"));
    }

    #[test]
    fn bus_error_invalid_state_for_resume() {
        // Can only resume a paused session
        let err = BusError::InvalidState {
            session_id: "sess-456".to_string(),
            current_state: "running".to_string(),
            expected_states: vec!["paused".to_string()],
        };

        let msg = err.to_string();
        assert!(msg.contains("running"));
        assert!(msg.contains("paused"));
    }

    #[test]
    fn bus_error_agent_not_found() {
        let err = BusError::AgentError("Agent 'unknown-agent' not found in vault".to_string());

        match &err {
            BusError::AgentError(msg) => assert!(msg.contains("unknown-agent")),
            _ => panic!("Expected AgentError"),
        }
    }

    #[test]
    fn bus_error_provider_not_configured() {
        let err =
            BusError::ProviderError("No API key configured for provider 'anthropic'".to_string());

        match &err {
            BusError::ProviderError(msg) => assert!(msg.contains("anthropic")),
            _ => panic!("Expected ProviderError"),
        }
    }

    #[test]
    fn bus_error_internal_from_string() {
        let err: BusError = "Database connection timeout".to_string().into();

        match err {
            BusError::Internal(msg) => assert!(msg.contains("timeout")),
            _ => panic!("Expected Internal error"),
        }
    }

    // ==================== Integration Scenario Tests ====================

    #[test]
    fn scenario_web_chat_new_session() {
        // User starts a new chat from the web UI
        let request = SessionRequest::new("root", "Hello, how can you help me?")
            .with_source(TriggerSource::Web);

        assert_eq!(request.source, TriggerSource::Web);
        assert!(request.session_id.is_none()); // New session
        assert!(request.external_ref.is_none()); // No external correlation
    }

    #[test]
    fn scenario_web_chat_continue_session() {
        // User sends a follow-up message
        let request = SessionRequest::new("root", "Can you explain that in more detail?")
            .with_source(TriggerSource::Web)
            .with_session_id("sess-abc123")
            .with_conversation_id("web-xyz789");

        assert!(request.session_id.is_some());
        assert!(request.conversation_id.is_some());
    }

    #[test]
    fn scenario_cron_scheduled_task() {
        // A cron job triggers a daily report
        let request = SessionRequest::new("report-generator", "Generate daily summary")
            .with_source(TriggerSource::Cron)
            .with_external_ref("cron:daily-report:2026-02-01")
            .with_priority(10)
            .with_metadata(serde_json::json!({
                "schedule": "0 0 * * *",
                "timezone": "UTC"
            }));

        assert_eq!(request.source, TriggerSource::Cron);
        assert_eq!(request.priority, Some(10));
    }

    #[test]
    fn scenario_api_webhook_handler() {
        // An external webhook triggers an agent
        let request = SessionRequest::new("github-bot", "New issue opened: Fix login bug")
            .with_source(TriggerSource::Api)
            .with_external_ref("github:issues:owner/repo#123")
            .with_metadata(serde_json::json!({
                "event": "issues.opened",
                "repo": "owner/repo",
                "issue_number": 123,
                "author": "user123"
            }));

        assert_eq!(request.source, TriggerSource::Api);
        assert!(request.external_ref.is_some());
    }

    #[test]
    fn scenario_connector_email_processor() {
        // An external connector processes an incoming email
        let request = SessionRequest::new("email-assistant", "Process this email")
            .with_source(TriggerSource::Connector)
            .with_external_ref("email:msg-id-abc123")
            .with_metadata(serde_json::json!({
                "from": "sender@example.com",
                "subject": "Meeting request",
                "received_at": "2026-02-01T10:30:00Z"
            }));

        assert_eq!(request.source, TriggerSource::Connector);
    }

    #[test]
    fn scenario_cli_interactive() {
        // User runs agent from CLI
        let request = SessionRequest::new("root", "What's the status of the project?")
            .with_source(TriggerSource::Cli);

        assert_eq!(request.source, TriggerSource::Cli);
        assert!(request.priority.is_none()); // CLI is typically interactive, no queue
    }
}
