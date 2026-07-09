//! # Stream Context
//!
//! Context struct for stream event processing during agent execution.

use api_logs::LogService;
use execution_state::StateService;
use gateway_events::EventBus;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use zbot_stores_sqlite::DatabaseManager;

use super::super::delegation::DelegationRequest;
use super::batch_writer::BatchWriterHandle;

// ============================================================================
// STREAM CONTEXT
// ============================================================================

/// Context for stream event processing.
///
/// Contains all the identifiers and services needed to process
/// stream events during agent execution.
#[derive(Clone)]
pub struct StreamContext {
    /// Agent ID
    pub agent_id: String,
    /// Conversation ID (for gateway events)
    pub conversation_id: String,
    /// Session ID
    pub session_id: String,
    /// Execution ID
    pub execution_id: String,
    /// Event bus for broadcasting events
    pub event_bus: Arc<EventBus>,
    /// Log service for execution tracing
    pub log_service: Arc<LogService<DatabaseManager>>,
    /// State service for token tracking
    pub state_service: Arc<StateService<DatabaseManager>>,
    /// Channel for delegation requests
    pub delegation_tx: mpsc::UnboundedSender<DelegationRequest>,
    /// Batch writer for non-blocking DB writes (token updates, logs)
    pub batch_writer: Option<BatchWriterHandle>,
    /// Vault directory root — needed for ward scaffolding at creation time
    pub vault_dir: PathBuf,
    /// Skills recommended by intent analysis — used to scope ward scaffolding
    pub recommended_skills: Vec<String>,
}

impl StreamContext {
    /// Create a new stream context.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_id: String,
        conversation_id: String,
        session_id: String,
        execution_id: String,
        event_bus: Arc<EventBus>,
        log_service: Arc<LogService<DatabaseManager>>,
        state_service: Arc<StateService<DatabaseManager>>,
        delegation_tx: mpsc::UnboundedSender<DelegationRequest>,
        vault_dir: PathBuf,
    ) -> Self {
        Self {
            agent_id,
            conversation_id,
            session_id,
            execution_id,
            event_bus,
            log_service,
            state_service,
            delegation_tx,
            batch_writer: None,
            vault_dir,
            recommended_skills: Vec::new(),
        }
    }

    /// Attach a batch writer for non-blocking DB writes.
    pub fn with_batch_writer(mut self, writer: BatchWriterHandle) -> Self {
        self.batch_writer = Some(writer);
        self
    }

    /// Set recommended skills from intent analysis — scopes ward scaffolding.
    pub fn with_recommended_skills(mut self, skills: Vec<String>) -> Self {
        self.recommended_skills = skills;
        self
    }
}
