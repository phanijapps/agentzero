//! # Stream Context
//!
//! Context struct for stream event processing during agent execution.

use api_logs::LogService;
use execution_state::StateService;
use gateway_events::EventBus;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tokio::sync::mpsc;
use zbot_runtime_sqlite::DatabaseManager;

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
    /// Surface ids created during this execution. Keeps projections additive:
    /// the first descriptor creates, later descriptors update.
    pub surface_ids: Arc<Mutex<HashSet<String>>>,
    /// Pending `write_file` calls into the ward's `outputs/` (the deliverable
    /// convention), keyed by tool-call id. On success the processor declares
    /// each as a goal artifact — so deliverables stay visible even when the
    /// model's respond omits them (sess-22816ad4 variance).
    pub output_write_calls: Arc<Mutex<HashMap<String, String>>>,
    /// (provider, model) identity for trace attribution — tool_call and
    /// tool_result events carry it so error rates split per model.
    pub model_info: Option<(String, String)>,
    /// Durable fact store for the in-session reflexion discharge
    /// (recovered failures → pattern facts at respond).
    pub memory_store: Option<Arc<dyn zbot_stores_traits::MemoryFactStore>>,
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
            surface_ids: Arc::new(Mutex::new(HashSet::new())),
            output_write_calls: Arc::new(Mutex::new(HashMap::new())),
            model_info: None,
            memory_store: None,
        }
    }

    /// Stamp the (provider, model) identity on every trace event this
    /// context emits.
    pub fn with_model_info(mut self, model_info: Option<(String, String)>) -> Self {
        self.model_info = model_info;
        self
    }

    /// Wire the fact store used by the reflexion discharge.
    pub fn with_memory_store(
        mut self,
        memory_store: Option<Arc<dyn zbot_stores_traits::MemoryFactStore>>,
    ) -> Self {
        self.memory_store = memory_store;
        self
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
