//! # WebSocket Handler
//!
//! Handles WebSocket connections and message routing.

use super::session::SessionRegistry;
use super::subscriptions::{
    EventMetadata, SessionScopeState, SubscribeError, SubscribeResult, SubscriptionManager,
};
use super::{ClientMessage, ServerMessage, SubscriptionErrorCode, SubscriptionScope};
use crate::error::Result;
use crate::events::{EventBus, GatewayEvent};
use crate::hooks::HookContext;
use crate::services::RuntimeService;
use execution_state::{DelegationType, ExecutionFilter};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// WebSocket handler for managing connections.
pub struct WebSocketHandler {
    event_bus: Arc<EventBus>,
    sessions: Arc<SessionRegistry>,
    runtime: Arc<RuntimeService>,
    subscriptions: Arc<SubscriptionManager>,
    agent_surfaces_enabled: bool,
    agent_tasks: Option<Arc<crate::durable_agent_tasks::DurableAgentTaskService>>,
}

impl WebSocketHandler {
    /// Create a new WebSocket handler.
    pub fn new(event_bus: Arc<EventBus>, runtime: Arc<RuntimeService>) -> Self {
        Self::new_with_surfaces(event_bus, runtime, false)
    }

    pub fn new_with_surfaces(
        event_bus: Arc<EventBus>,
        runtime: Arc<RuntimeService>,
        agent_surfaces_enabled: bool,
    ) -> Self {
        Self {
            event_bus,
            sessions: Arc::new(SessionRegistry::new()),
            runtime,
            subscriptions: Arc::new(SubscriptionManager::new()),
            agent_surfaces_enabled,
            agent_tasks: None,
        }
    }

    pub fn with_agent_tasks(
        mut self,
        agent_tasks: Arc<crate::durable_agent_tasks::DurableAgentTaskService>,
    ) -> Self {
        self.agent_tasks = Some(agent_tasks);
        self
    }

    pub fn agent_tasks(&self) -> Option<Arc<crate::durable_agent_tasks::DurableAgentTaskService>> {
        self.agent_tasks.clone()
    }

    /// Get the subscription manager.
    pub fn subscriptions(&self) -> Arc<SubscriptionManager> {
        self.subscriptions.clone()
    }

    /// Get the session registry.
    pub fn sessions(&self) -> Arc<SessionRegistry> {
        self.sessions.clone()
    }

    /// Get the runtime service. Exposed so the Axum-path handler can
    /// dispatch incoming client messages through the shared forwarder.
    pub fn runtime(&self) -> Arc<RuntimeService> {
        self.runtime.clone()
    }

    /// Spawn the two background tasks that support the `/ws` Axum route:
    ///
    ///   1. **Subscription cleanup** — evicts stale clients every 30s.
    ///   2. **Event router** — drains the `EventBus` and routes each event
    ///      through `SubscriptionManager` to every subscribed client.
    ///
    /// These tasks remain independent of the connection driver so
    /// `Server::start` can spawn them before the HTTP listener accepts
    /// WebSocket upgrades.
    pub fn spawn_background_tasks(&self, shutdown: &broadcast::Sender<()>) {
        let cleanup_subscriptions = self.subscriptions.clone();
        let mut cleanup_shutdown = shutdown.subscribe();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let cleaned = cleanup_subscriptions
                            .cleanup_stale_clients(std::time::Duration::from_secs(120))
                            .await;
                        if cleaned > 0 {
                            info!("Cleaned up {} stale WebSocket clients", cleaned);
                        }
                    }
                    _ = cleanup_shutdown.recv() => {
                        debug!("Subscription cleanup task shutting down");
                        break;
                    }
                }
            }
        });

        let router_subscriptions = self.subscriptions.clone();
        let router_runtime = self.runtime.clone();
        let agent_surfaces_enabled = self.agent_surfaces_enabled;
        let mut router_event_rx = self.event_bus.subscribe_all();
        let mut router_shutdown = shutdown.subscribe();
        tokio::spawn(async move {
            Self::run_event_router(
                router_subscriptions,
                router_runtime,
                agent_surfaces_enabled,
                &mut router_event_rx,
                &mut router_shutdown,
            )
            .await;
        });
    }

    /// Event-router loop extracted so the spawn site above stays readable.
    async fn run_event_router(
        router_subscriptions: Arc<SubscriptionManager>,
        router_runtime: Arc<RuntimeService>,
        agent_surfaces_enabled: bool,
        router_event_rx: &mut tokio::sync::broadcast::Receiver<GatewayEvent>,
        router_shutdown: &mut broadcast::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                result = router_event_rx.recv() => {
                    match result {
                        Ok(event) => {
                            if !agent_surfaces_enabled && matches!(event, GatewayEvent::SurfaceCreated { .. } | GatewayEvent::SurfaceUpdated { .. } | GatewayEvent::SurfaceDeleted { .. } | GatewayEvent::SurfaceValidationFailed { .. }) {
                                continue;
                            }
                            match &event {
                                GatewayEvent::DelegationCompleted { session_id, child_agent_id, .. } => {
                                    tracing::debug!(session_id = %session_id, agent = %child_agent_id, "Delegation completed");
                                }
                                GatewayEvent::DelegationStarted { session_id, child_agent_id, .. } => {
                                    tracing::debug!(session_id = %session_id, agent = %child_agent_id, "Delegation started");
                                }
                                _ => {}
                            }

                            // Check for new root executions on AgentStarted events
                            // If a new root is detected, update the cache for session-scoped subscribers
                            if let GatewayEvent::AgentStarted { session_id, execution_id, conversation_id, .. } = &event {
                                info!(
                                    session_id = %session_id,
                                    execution_id = %execution_id,
                                    conversation_id = ?conversation_id,
                                    "AgentStarted event received"
                                );
                                // Look up the execution to check if it's a root
                                if let Some(runner) = router_runtime.runner() {
                                    let state_service = runner.state_service();
                                    if let Ok(Some(execution)) = state_service.get_execution(execution_id) {
                                        // Root executions have no parent_execution_id
                                        if execution.parent_execution_id.is_none() {
                                            info!(
                                                session_id = %session_id,
                                                execution_id = %execution_id,
                                                conversation_id = ?conversation_id,
                                                "New root execution detected, updating scope caches"
                                            );
                                            // Update cache for session_id subscribers
                                            router_subscriptions
                                                .add_root_to_caches(session_id, execution_id)
                                                .await;
                                                        }
                                    }
                                }
                            }

                            // Extract metadata for scope-based filtering
                            let metadata = gateway_event_to_metadata(&event);

                            // Route events by session_id only.
                            // Clients auto-subscribe to session_id on invoke, so all events
                            // are delivered through a single path (no duplicates).
                            let session_id = event.session_id().map(|s| s.to_string());

                            if let Some(server_msg) = gateway_event_to_server_message(event) {
                                let mut total_sent = 0u64;

                                // Route by session_id (for session-based subscriptions)
                                // Uses scoped routing - filters based on subscriber's scope
                                if let Some(ref sid) = session_id {
                                    let result = router_subscriptions
                                        .route_event_scoped(sid, server_msg.clone(), &metadata)
                                        .await;
                                    total_sent += result.sent;

                                    if result.sent > 0 {
                                        debug!(
                                            session_id = %sid,
                                            sent = result.sent,
                                            "Routed event by session_id (scoped)"
                                        );
                                    }
                                }

                                // If no subscribers found, log at debug — this is
                                // expected for cron-fired sessions (no UI client
                                // subscribed) and for sessions where the user closed
                                // the panel mid-execution. Not an error.
                                if total_sent == 0 && session_id.is_some() {
                                    debug!(
                                        session_id = ?session_id,
                                        "No subscribers found for event"
                                    );
                                } else if total_sent > 0 {
                                    debug!(
                                        session_id = ?session_id,
                                        total_sent = total_sent,
                                        "Event routed successfully"
                                    );
                                }

                                // Global events (like Pong) with no identifiers - broadcast to all
                                if session_id.is_none() {
                                    router_subscriptions.broadcast_global(server_msg).await;
                                }
                            }
                        }
                        Err(e) => {
                            error!("Event router receive error: {}", e);
                            break;
                        }
                    }
                }
                _ = router_shutdown.recv() => {
                    debug!("Event router shutting down");
                    break;
                }
            }
        }
    }

    /// Broadcast a message to all sessions for an agent.
    pub async fn broadcast_to_agent(&self, agent_id: &str, msg: ServerMessage) {
        self.sessions.broadcast_to_agent(agent_id, msg).await;
    }
}

/// Handle a client message.
pub(super) async fn handle_client_message(
    session_id: &str,
    msg: ClientMessage,
    sessions: &SessionRegistry,
    runtime: &RuntimeService,
    subscriptions: Arc<SubscriptionManager>,
    agent_tasks: Option<Arc<crate::durable_agent_tasks::DurableAgentTaskService>>,
) -> Result<()> {
    match msg {
        ClientMessage::Invoke {
            agent_id,
            conversation_id,
            message,
            session_id: exec_session_id,
            metadata,
            mode,
        } => {
            debug!(
                "Session {} invoking agent {} conversation {} (exec_session: {:?})",
                session_id, agent_id, conversation_id, exec_session_id
            );

            // Create hook context for WebSocket connection
            let mut hook_context = HookContext::web(session_id);
            hook_context.metadata.insert(
                "conversation_id".to_string(),
                serde_json::Value::String(conversation_id.clone()),
            );

            // Build callback that subscribes the WS client before any events fire.
            // This ensures IntentAnalysisStarted/Complete reach the subscriber.
            let subs = subscriptions.clone();
            let ws_sid = session_id.to_string();
            let on_ready: gateway_execution::OnSessionReady =
                Box::new(move |agent_session_id: String| {
                    Box::pin(async move {
                        let _ = subs
                            .subscribe_with_scope(
                                &ws_sid,
                                agent_session_id,
                                SubscriptionScope::Session,
                                Some(SessionScopeState::default()),
                            )
                            .await;
                    })
                });

            // Invoke the agent via runtime service with hook context and callback.
            let invoke_mode = normalized_invoke_mode(&conversation_id, &mode);
            // The invoke envelope can carry arbitrary metadata from untrusted
            // clients. This path needs only the opaque correlation id, so do
            // not propagate unrelated keys into execution configuration.
            let client_message_id = client_message_id_from_metadata(metadata);
            if invoke_mode.as_deref() == Some("research") {
                let Some(agent_tasks) = agent_tasks else {
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::error(
                            Some(conversation_id),
                            "invocation_failed",
                            "Unable to start this request",
                        ));
                    }
                    return Ok(());
                };
                let message_id =
                    crate::durable_agent_tasks::canonical_message_id(client_message_id.as_deref());
                let reserved_session_id = exec_session_id.clone().unwrap_or_else(|| {
                    crate::durable_agent_tasks::reserved_session_id(&message_id)
                });
                let already_subscribed = subscriptions
                    .is_subscribed(&session_id.to_string(), &reserved_session_id)
                    .await;
                if !already_subscribed
                    && subscriptions
                        .subscribe_with_scope(
                            &session_id.to_string(),
                            reserved_session_id,
                            SubscriptionScope::Session,
                            Some(SessionScopeState::default()),
                        )
                        .await
                        .is_err()
                {
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::error(
                            Some(conversation_id),
                            "invocation_failed",
                            "Unable to start this request",
                        ));
                    }
                    return Ok(());
                }
                match agent_tasks
                    .enqueue_research(
                        session_id,
                        &agent_id,
                        &conversation_id,
                        &message,
                        exec_session_id.as_deref(),
                        Some(&message_id),
                    )
                    .await
                {
                    Ok(receipt) => {
                        if let Some(client) = sessions.get(session_id).await {
                            tokio::spawn(async move {
                                let ready = agent_tasks.wait_until_ready(&receipt).await;
                                let response = if ready.is_ok() {
                                    ServerMessage::InvokeAccepted {
                                        session_id: receipt.session_id,
                                        conversation_id: conversation_id.clone(),
                                    }
                                } else {
                                    ServerMessage::error(
                                        Some(conversation_id),
                                        "invocation_failed",
                                        "Unable to start this request",
                                    )
                                };
                                let _ = client.send(response);
                            });
                        }
                    }
                    Err(_) => {
                        if let Some(session) = sessions.get(session_id).await {
                            let _ = session.send(ServerMessage::error(
                                Some(conversation_id),
                                "invocation_failed",
                                "Unable to start this request",
                            ));
                        }
                    }
                }
                return Ok(());
            }
            // Direct invocations keep the existing continuation subscription
            // behavior. New direct sessions subscribe in `on_ready`.
            if let Some(ref sid) = exec_session_id {
                let _ = subscriptions
                    .subscribe_with_scope(
                        &session_id.to_string(),
                        sid.clone(),
                        SubscriptionScope::Session,
                        Some(SessionScopeState::default()),
                    )
                    .await;
            }
            match runtime
                .invoke_with_hook_and_callback(
                    &agent_id,
                    &conversation_id,
                    &message,
                    hook_context,
                    exec_session_id,
                    Some(on_ready),
                    invoke_mode,
                    client_message_id,
                )
                .await
            {
                Ok((_handle, returned_session_id)) => {
                    debug!(
                        "Agent {} invocation started for conversation {} (session: {})",
                        agent_id, conversation_id, returned_session_id
                    );

                    // Notify client of the session_id so it can update its state
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::InvokeAccepted {
                            session_id: returned_session_id,
                            conversation_id,
                        });
                    }
                }
                Err(e) => {
                    warn!("Failed to invoke agent {}: {}", agent_id, e);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::error(
                            Some(conversation_id),
                            "invocation_failed",
                            &e,
                        ));
                    }
                }
            }
        }
        ClientMessage::Stop { conversation_id } => {
            debug!(
                "Session {} stopping conversation {}",
                session_id, conversation_id
            );

            match runtime.stop(&conversation_id).await {
                Ok(()) => {
                    debug!("Stop requested for conversation {}", conversation_id);
                }
                Err(e) => {
                    warn!("Failed to stop conversation {}: {}", conversation_id, e);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::error(
                            Some(conversation_id),
                            "stop_failed",
                            &e,
                        ));
                    }
                }
            }
        }
        ClientMessage::Continue {
            conversation_id,
            additional_iterations,
        } => {
            debug!(
                "Session {} continuing conversation {} with {} more iterations",
                session_id, conversation_id, additional_iterations
            );

            match runtime
                .continue_execution(&conversation_id, additional_iterations)
                .await
            {
                Ok(()) => {
                    debug!(
                        "Continuation requested for conversation {}",
                        conversation_id
                    );
                }
                Err(e) => {
                    warn!("Failed to continue conversation {}: {}", conversation_id, e);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::error(
                            Some(conversation_id),
                            "continue_failed",
                            &e,
                        ));
                    }
                }
            }
        }
        ClientMessage::Ping => {
            // Update last activity to prevent stale cleanup
            subscriptions.touch_client(&session_id.to_string()).await;
            if let Some(session) = sessions.get(session_id).await {
                let _ = session.send(ServerMessage::Pong);
            }
        }
        ClientMessage::Pause {
            session_id: exec_session_id,
        } => {
            debug!(
                "Session {} pausing execution session {}",
                session_id, exec_session_id
            );

            match runtime.pause(&exec_session_id).await {
                Ok(()) => {
                    debug!("Execution session {} paused", exec_session_id);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::SessionPaused {
                            session_id: exec_session_id,
                        });
                    }
                }
                Err(e) => {
                    warn!("Failed to pause session {}: {}", exec_session_id, e);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::error(None, "pause_failed", &e));
                    }
                }
            }
        }
        ClientMessage::Resume {
            session_id: exec_session_id,
        } => {
            debug!(
                "Session {} resuming execution session {}",
                session_id, exec_session_id
            );

            match runtime.resume(&exec_session_id).await {
                Ok(()) => {
                    debug!("Execution session {} resumed", exec_session_id);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::SessionResumed {
                            session_id: exec_session_id,
                        });
                    }
                }
                Err(e) => {
                    warn!("Failed to resume session {}: {}", exec_session_id, e);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::error(None, "resume_failed", &e));
                    }
                }
            }
        }
        ClientMessage::Cancel {
            session_id: exec_session_id,
        } => {
            debug!(
                "Session {} cancelling execution session {}",
                session_id, exec_session_id
            );

            match runtime.cancel(&exec_session_id).await {
                Ok(()) => {
                    debug!("Execution session {} cancelled", exec_session_id);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::SessionCancelled {
                            session_id: exec_session_id,
                        });
                    }
                }
                Err(e) => {
                    warn!("Failed to cancel session {}: {}", exec_session_id, e);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::error(None, "cancel_failed", &e));
                    }
                }
            }
        }
        ClientMessage::EndSession {
            session_id: exec_session_id,
        } => {
            debug!(
                "Session {} ending execution session {}",
                session_id, exec_session_id
            );

            match runtime.end_session(&exec_session_id).await {
                Ok(()) => {
                    info!("Execution session {} ended (completed)", exec_session_id);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::SessionEnded {
                            session_id: exec_session_id.clone(),
                        });
                    }
                }
                Err(e) => {
                    warn!("Failed to end session {}: {}", exec_session_id, e);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::error(None, "end_session_failed", &e));
                    }
                }
            }
        }
        ClientMessage::Subscribe {
            conversation_id,
            scope,
        } => {
            debug!(
                "Session {} subscribing to conversation {} with scope {:?}",
                session_id, conversation_id, scope
            );

            // Update last activity to prevent stale cleanup
            subscriptions.touch_client(&session_id.to_string()).await;

            // For Session scope, query root execution IDs from state service
            let (scope_state, root_ids_for_response) =
                if matches!(scope, SubscriptionScope::Session) {
                    // conversation_id is actually the session_id in our data model
                    if let Some(runner) = runtime.runner() {
                        let state_service = runner.state_service();

                        // Query root executions for this session
                        let filter = ExecutionFilter {
                            session_id: Some(conversation_id.clone()),
                            ..Default::default()
                        };

                        match state_service.list_executions(&filter) {
                            Ok(executions) => {
                                // Filter to root executions only (parent_execution_id is None and delegation_type is Root)
                                let root_ids: HashSet<String> = executions
                                    .into_iter()
                                    .filter(|exec| {
                                        exec.parent_execution_id.is_none()
                                            && exec.delegation_type == DelegationType::Root
                                    })
                                    .map(|exec| exec.id)
                                    .collect();

                                debug!(
                                    "Found {} root executions for session {} scope subscription",
                                    root_ids.len(),
                                    conversation_id
                                );

                                let scope_state = SessionScopeState::new(root_ids.clone());
                                (Some(scope_state), Some(root_ids))
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to query root executions for session {}: {}",
                                    conversation_id, e
                                );
                                // Fall back to empty cache - events will still work, just without filtering
                                (Some(SessionScopeState::default()), Some(HashSet::new()))
                            }
                        }
                    } else {
                        warn!("Runtime not initialized - cannot query root executions");
                        (Some(SessionScopeState::default()), Some(HashSet::new()))
                    }
                } else {
                    // For All or Execution scopes, no session scope state needed
                    (None, None)
                };

            // Subscribe with scope and state
            match subscriptions
                .subscribe_with_scope(
                    &session_id.to_string(),
                    conversation_id.clone(),
                    scope.clone(),
                    scope_state,
                )
                .await
            {
                Ok(SubscribeResult::Subscribed { current_sequence }) => {
                    debug!(
                        "Session {} subscribed to {} (seq: {}, scope: {:?})",
                        session_id, conversation_id, current_sequence, scope
                    );
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::Subscribed {
                            conversation_id,
                            current_sequence,
                            root_execution_ids: root_ids_for_response,
                        });
                    }
                }
                Ok(SubscribeResult::AlreadySubscribed { current_sequence }) => {
                    debug!(
                        "Session {} already subscribed to {} (seq: {}, scope: {:?})",
                        session_id, conversation_id, current_sequence, scope
                    );
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::Subscribed {
                            conversation_id,
                            current_sequence,
                            root_execution_ids: root_ids_for_response,
                        });
                    }
                }
                Err(SubscribeError::ClientNotFound) => {
                    warn!("Subscribe failed: client {} not found", session_id);
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::subscription_error(
                            &conversation_id,
                            SubscriptionErrorCode::ServerError,
                            "Client not registered",
                        ));
                    }
                }
                Err(SubscribeError::TooManySubscriptions { limit }) => {
                    warn!(
                        "Subscribe failed: client {} exceeded limit of {}",
                        session_id, limit
                    );
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::subscription_error(
                            &conversation_id,
                            SubscriptionErrorCode::LimitExceeded,
                            &format!("Maximum {} subscriptions per client", limit),
                        ));
                    }
                }
                Err(SubscribeError::ConversationFull { limit }) => {
                    warn!(
                        "Subscribe failed: conversation {} at limit of {}",
                        conversation_id, limit
                    );
                    if let Some(session) = sessions.get(session_id).await {
                        let _ = session.send(ServerMessage::subscription_error(
                            &conversation_id,
                            SubscriptionErrorCode::LimitExceeded,
                            &format!("Conversation has maximum {} subscribers", limit),
                        ));
                    }
                }
            }
        }
        ClientMessage::Unsubscribe { conversation_id } => {
            debug!(
                "Session {} unsubscribing from conversation {}",
                session_id, conversation_id
            );

            // Update last activity to prevent stale cleanup
            subscriptions.touch_client(&session_id.to_string()).await;
            subscriptions
                .unsubscribe(&session_id.to_string(), &conversation_id)
                .await;

            if let Some(session) = sessions.get(session_id).await {
                let _ = session.send(ServerMessage::Unsubscribed { conversation_id });
            }
        }
        ClientMessage::PresentationCapabilities { catalogs } => {
            subscriptions
                .set_surface_catalogs(&session_id.to_string(), catalogs.into_iter().collect())
                .await;
        }
    }

    Ok(())
}

pub(crate) fn gateway_event_to_server_message(event: GatewayEvent) -> Option<ServerMessage> {
    match event {
        GatewayEvent::SurfaceCreated {
            session_id,
            execution_id,
            surface,
        } => Some(ServerMessage::SurfaceCreated {
            session_id,
            execution_id,
            surface: serde_json::to_value(surface).expect("work surface is serializable"),
            seq: None,
        }),
        GatewayEvent::SurfaceUpdated {
            session_id,
            execution_id,
            surface,
        } => Some(ServerMessage::SurfaceUpdated {
            session_id,
            execution_id,
            surface: serde_json::to_value(surface).expect("work surface is serializable"),
            seq: None,
        }),
        GatewayEvent::SurfaceDeleted {
            session_id,
            execution_id,
            surface_id,
        } => Some(ServerMessage::SurfaceDeleted {
            session_id,
            execution_id,
            surface_id,
            seq: None,
        }),
        GatewayEvent::SurfaceValidationFailed {
            session_id,
            execution_id,
            surface_id,
            reason,
        } => Some(ServerMessage::SurfaceValidationFailed {
            session_id,
            execution_id,
            surface_id,
            reason,
            seq: None,
        }),
        GatewayEvent::AgentStarted {
            agent_id,
            session_id,
            execution_id,
            conversation_id,
            ..
        } => Some(ServerMessage::AgentStarted {
            agent_id,
            session_id,
            execution_id,
            conversation_id,
            seq: None,
        }),
        GatewayEvent::AgentCompleted {
            agent_id,
            session_id,
            execution_id,
            result,
            conversation_id,
            ..
        } => Some(ServerMessage::AgentCompleted {
            agent_id,
            session_id,
            execution_id,
            conversation_id,
            result,
            seq: None,
        }),
        GatewayEvent::AgentStopped {
            agent_id,
            session_id,
            execution_id,
            iteration,
            conversation_id,
            ..
        } => Some(ServerMessage::AgentStopped {
            agent_id,
            session_id,
            execution_id,
            conversation_id,
            iteration,
            seq: None,
        }),
        GatewayEvent::Token {
            session_id,
            execution_id,
            delta,
            conversation_id,
            ..
        } => Some(ServerMessage::Token {
            session_id,
            execution_id,
            conversation_id,
            delta,
            seq: None,
        }),
        GatewayEvent::Thinking {
            session_id,
            execution_id,
            content,
            conversation_id,
            ..
        } => Some(ServerMessage::Thinking {
            session_id,
            execution_id,
            conversation_id,
            content,
            seq: None,
        }),
        GatewayEvent::ToolCall {
            session_id,
            execution_id,
            tool_id,
            tool_name,
            args,
            conversation_id,
            ..
        } => Some(ServerMessage::ToolCall {
            session_id,
            execution_id,
            conversation_id,
            tool_call_id: tool_id,
            tool: tool_name,
            args,
            seq: None,
        }),
        GatewayEvent::ToolResult {
            session_id,
            execution_id,
            tool_id,
            result,
            error,
            conversation_id,
            ..
        } => Some(ServerMessage::ToolResult {
            session_id,
            execution_id,
            conversation_id,
            tool_call_id: tool_id,
            result,
            error,
            seq: None,
        }),
        GatewayEvent::TurnComplete {
            session_id,
            execution_id,
            message,
            conversation_id,
            ..
        } => Some(ServerMessage::TurnComplete {
            session_id,
            execution_id,
            conversation_id,
            final_message: Some(message),
            seq: None,
        }),
        GatewayEvent::Error {
            session_id,
            execution_id,
            message,
            conversation_id,
            ..
        } => Some(ServerMessage::Error {
            session_id,
            execution_id,
            conversation_id,
            code: "execution_error".to_string(),
            message,
            seq: None,
        }),
        GatewayEvent::IterationUpdate {
            session_id,
            execution_id,
            current,
            max,
            conversation_id,
            ..
        } => Some(ServerMessage::Iteration {
            session_id,
            execution_id,
            conversation_id,
            current,
            max,
            seq: None,
        }),
        GatewayEvent::ContinuationPrompt {
            session_id,
            execution_id,
            iteration,
            message,
            conversation_id,
            ..
        } => Some(ServerMessage::ContinuationPrompt {
            session_id,
            execution_id,
            conversation_id,
            iteration,
            message,
            seq: None,
        }),

        // Respond events are handled by the hook system, not WebSocket directly
        GatewayEvent::Respond {
            session_id,
            execution_id,
            message,
            conversation_id,
            ..
        } => Some(ServerMessage::TurnComplete {
            session_id,
            execution_id,
            conversation_id,
            final_message: Some(message),
            seq: None,
        }),

        // Delegation events - sent to frontend for UI updates
        GatewayEvent::DelegationStarted {
            session_id,
            parent_execution_id,
            child_execution_id,
            parent_agent_id,
            child_agent_id,
            task,
            parent_conversation_id,
            child_conversation_id,
        } => Some(ServerMessage::DelegationStarted {
            session_id,
            parent_execution_id,
            child_execution_id,
            parent_agent_id,
            child_agent_id,
            task,
            parent_conversation_id,
            child_conversation_id,
            seq: None,
        }),

        GatewayEvent::DelegationCompleted {
            session_id,
            parent_execution_id,
            child_execution_id,
            parent_agent_id,
            child_agent_id,
            result,
            parent_conversation_id,
            child_conversation_id,
        } => Some(ServerMessage::DelegationCompleted {
            session_id,
            parent_execution_id,
            child_execution_id,
            parent_agent_id,
            child_agent_id,
            result,
            parent_conversation_id,
            child_conversation_id,
            seq: None,
        }),

        // Heartbeat signals execution alive during silent phases (LLM reasoning)
        GatewayEvent::Heartbeat {
            session_id,
            execution_id,
            conversation_id,
        } => Some(ServerMessage::Heartbeat {
            session_id,
            execution_id,
            conversation_id,
            seq: None,
        }),

        // Internal continuation events are handled by the system, not WebSocket
        GatewayEvent::SessionContinuationReady { .. } => None,

        // New message added - notify frontend to refresh
        GatewayEvent::MessageAdded {
            session_id,
            execution_id,
            role,
            content,
            conversation_id,
            ..
        } => Some(ServerMessage::MessageAdded {
            session_id,
            execution_id,
            conversation_id,
            role,
            content,
            seq: None,
        }),

        // Token usage update for real-time metrics
        GatewayEvent::TokenUsage {
            session_id,
            execution_id,
            tokens_in,
            tokens_out,
            conversation_id,
            ..
        } => Some(ServerMessage::TokenUsage {
            session_id,
            execution_id,
            conversation_id,
            tokens_in,
            tokens_out,
            seq: None,
        }),

        // Ward changed - agent switched project directory
        GatewayEvent::WardChanged {
            session_id,
            execution_id,
            ward_id,
        } => Some(ServerMessage::WardChanged {
            session_id,
            execution_id,
            ward_id,
            seq: None,
        }),

        // Iterations auto-extended by executor
        GatewayEvent::IterationsExtended {
            session_id,
            execution_id,
            iterations_used,
            iterations_added,
            reason,
            conversation_id,
        } => Some(ServerMessage::IterationsExtended {
            session_id,
            execution_id,
            iterations_used,
            iterations_added,
            reason,
            conversation_id,
            seq: None,
        }),

        // Plan update from update_plan tool
        GatewayEvent::PlanUpdate {
            session_id,
            execution_id,
            plan,
            explanation,
            conversation_id,
        } => Some(ServerMessage::PlanUpdate {
            session_id,
            execution_id,
            plan,
            explanation,
            conversation_id,
            seq: None,
        }),

        // Intent analysis started — show "Analyzing..." in UI
        gateway_events::GatewayEvent::IntentAnalysisStarted {
            session_id,
            execution_id,
        } => Some(ServerMessage::IntentAnalysisStarted {
            session_id,
            execution_id,
            seq: None,
        }),

        // Intent analysis complete — forwarded so UI sidebar can display results
        GatewayEvent::IntentAnalysisComplete {
            session_id,
            execution_id,
            primary_intent,
            hidden_intents,
            recommended_skills,
            recommended_agents,
            ward_recommendation,
            execution_strategy,
        } => Some(ServerMessage::IntentAnalysisComplete {
            session_id,
            execution_id,
            primary_intent,
            hidden_intents,
            recommended_skills,
            recommended_agents,
            ward_recommendation,
            execution_strategy,
            seq: None,
        }),

        // Session title changed
        GatewayEvent::SessionTitleChanged { session_id, title } => {
            Some(ServerMessage::SessionTitleChanged {
                session_id,
                title,
                seq: None,
            })
        }

        // Intent analysis skipped (continuation of existing session)
        GatewayEvent::IntentAnalysisSkipped {
            session_id,
            execution_id,
        } => Some(ServerMessage::IntentAnalysisSkipped {
            session_id,
            execution_id,
            seq: None,
        }),

        // Customization file change events are UI-only (delivered via the
        // /api/customization SSE stream, not the session WebSocket).
        GatewayEvent::CustomizationFileChanged { .. } => None,

        // RecallTrace — broadcast globally so /observatory-v2 can light
        // up the clusters the agent just consulted.
        GatewayEvent::RecallTrace {
            agent_id,
            conversation_id,
            seed_entity_ids,
            seed_aggregate_ids,
            lca_aggregate_id,
            surfaced_item_count,
            match_sources: _,
            ranking_reasons: _,
            degraded_reasons: _,
            embedding_provider_identity: _,
            taxonomy_expansion,
        } => Some(ServerMessage::RecallTrace {
            agent_id,
            conversation_id,
            seed_entity_ids,
            seed_aggregate_ids,
            lca_aggregate_id,
            surfaced_item_count,
            taxonomy_expansion,
            seq: None,
        }),
    }
}

fn normalized_invoke_mode(conversation_id: &str, mode: &str) -> Option<String> {
    if conversation_id.starts_with("research-") {
        return Some("research".to_string());
    }

    match mode {
        "deep" | "research" => Some("research".to_string()),
        other => Some(other.to_string()),
    }
}

/// Extract event metadata for scope-based filtering decisions.
///
/// Returns metadata indicating:
/// - The execution_id (if applicable)
/// - Whether this is a delegation lifecycle event (always shown in session scope)
fn gateway_event_to_metadata(event: &GatewayEvent) -> EventMetadata {
    // Check if this is a delegation lifecycle event
    let is_delegation_event = matches!(
        event,
        GatewayEvent::DelegationStarted { .. } | GatewayEvent::DelegationCompleted { .. }
    );

    EventMetadata {
        execution_id: event.execution_id().map(|s| s.to_string()),
        is_delegation_event,
    }
}

/// Retain only the correlation id used to reconcile an optimistic Research
/// turn with its durable root message. Validation happens in the execution
/// bootstrap, immediately before it can become a message-store primary key.
fn client_message_id_from_metadata(metadata: Option<serde_json::Value>) -> Option<String> {
    metadata?
        .get("client_message_id")?
        .as_str()
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::{client_message_id_from_metadata, handle_client_message, normalized_invoke_mode};
    use crate::server::GatewayServer;
    use crate::websocket::{ClientMessage, ServerMessage, SessionRegistry, SubscriptionManager};
    use crate::GatewayConfig;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    #[test]
    fn research_conversation_ids_force_research_mode() {
        assert_eq!(
            normalized_invoke_mode("research-abc", "fast"),
            Some("research".to_string())
        );
        assert_eq!(
            normalized_invoke_mode("research-abc", "chat"),
            Some("research".to_string())
        );
        assert_eq!(
            normalized_invoke_mode("research-abc", "deep"),
            Some("research".to_string())
        );
    }

    #[test]
    fn non_research_conversation_ids_preserve_explicit_chat_mode() {
        assert_eq!(
            normalized_invoke_mode("chat-abc", "fast"),
            Some("fast".to_string())
        );
        assert_eq!(
            normalized_invoke_mode("sess-chat-abc", "chat"),
            Some("chat".to_string())
        );
        assert_eq!(
            normalized_invoke_mode("chat-abc", "deep"),
            Some("research".to_string())
        );
    }

    #[test]
    fn invoke_metadata_forwards_only_the_client_message_id() {
        let client_message_id = client_message_id_from_metadata(Some(serde_json::json!({
            "client_message_id": "msg-550e8400-e29b-41d4-a716-446655440000",
            "unexpected": "must not enter execution config",
        })));
        assert_eq!(
            client_message_id.as_deref(),
            Some("msg-550e8400-e29b-41d4-a716-446655440000")
        );
        assert_eq!(
            client_message_id_from_metadata(Some(serde_json::json!({}))),
            None
        );
    }

    #[tokio::test]
    async fn research_subscribes_then_persists_before_acceptance() {
        let temp_dir = TempDir::new().unwrap();
        let server = GatewayServer::new(GatewayConfig::default(), temp_dir.path().to_path_buf());
        let sessions = SessionRegistry::new();
        let subscriptions = Arc::new(SubscriptionManager::new());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ws_session = crate::websocket::WsSession::new(tx.clone());
        let client_id = ws_session.id.clone();
        sessions.register(ws_session).await;
        subscriptions.connect(client_id.clone(), tx).await;
        let message_id = "msg-550e8400-e29b-41d4-a716-446655440000";
        let session_id = crate::durable_agent_tasks::reserved_session_id(message_id);
        let execution_id = crate::durable_agent_tasks::reserved_execution_id(message_id);
        let prompt = "Investigate durable handoff";
        let service = server.ws_handler().agent_tasks().unwrap();

        handle_client_message(
            &client_id,
            ClientMessage::Invoke {
                agent_id: "root".to_owned(),
                conversation_id: "research-1".to_owned(),
                message: prompt.to_owned(),
                session_id: None,
                metadata: Some(serde_json::json!({"client_message_id": message_id})),
                mode: "deep".to_owned(),
            },
            &sessions,
            &server.runtime(),
            subscriptions.clone(),
            Some(service.clone()),
        )
        .await
        .unwrap();

        assert!(subscriptions.is_subscribed(&client_id, &session_id).await);
        let duplicate = service
            .enqueue_research(
                &client_id,
                "root",
                "research-1",
                prompt,
                None,
                Some(message_id),
            )
            .await
            .unwrap();
        assert!(!duplicate.inserted, "invoke must persist before returning");
        assert!(rx.try_recv().is_err(), "acceptance must wait for bootstrap");

        let session = execution_state::Session::new_with_id(
            &session_id,
            "root",
            execution_state::TriggerSource::Web,
        )
        .unwrap();
        server
            .state()
            .state_service
            .create_session_from(&session)
            .unwrap();
        let execution =
            execution_state::AgentExecution::new_root_with_id(&execution_id, &session_id, "root")
                .unwrap();
        server
            .state()
            .state_service
            .create_execution(&execution)
            .unwrap();
        server
            .state()
            .messages
            .append(&zbot_conversation::Message {
                id: message_id.to_owned(),
                execution_id: Some(execution_id.clone()),
                session_id: session_id.clone(),
                role: "user".to_owned(),
                content: prompt.to_owned(),
                created_at: chrono::Utc::now().to_rfc3339(),
                token_count: 1,
                tool_calls: None,
                tool_call_id: None,
                seq: 0,
            })
            .unwrap();

        let accepted = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            accepted,
            ServerMessage::InvokeAccepted {
                session_id: accepted_session,
                conversation_id
            } if accepted_session == session_id && conversation_id == "research-1"
        ));
    }

    #[tokio::test]
    async fn research_subscription_failure_sends_normalized_error_without_enqueue() {
        const PROMPT_SENTINEL: &str = "PROMPT-SECRET-SENTINEL";
        const PROVIDER_SENTINEL: &str = "PROVIDER-RAW-DIAGNOSTIC-SENTINEL";
        let temp_dir = TempDir::new().unwrap();
        let server = GatewayServer::new(GatewayConfig::default(), temp_dir.path().to_path_buf());
        let sessions = SessionRegistry::new();
        let subscriptions = Arc::new(SubscriptionManager::new());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ws_session = crate::websocket::WsSession::new(tx);
        let client_id = ws_session.id.clone();
        sessions.register(ws_session).await;
        let message_id = "msg-550e8400-e29b-41d4-a716-446655440001";
        let service = server.ws_handler().agent_tasks().unwrap();

        handle_client_message(
            &client_id,
            ClientMessage::Invoke {
                agent_id: "root".to_owned(),
                conversation_id: "research-2".to_owned(),
                message: PROMPT_SENTINEL.to_owned(),
                session_id: None,
                metadata: Some(serde_json::json!({"client_message_id": message_id})),
                mode: "research".to_owned(),
            },
            &sessions,
            &server.runtime(),
            subscriptions,
            Some(service.clone()),
        )
        .await
        .unwrap();

        let response = rx.recv().await.unwrap();
        let encoded_response = serde_json::to_string(&response).unwrap();
        assert!(matches!(
            response,
            ServerMessage::Error { code, message, .. }
                if code == "invocation_failed" && message == "Unable to start this request"
        ));
        for secret in [PROMPT_SENTINEL, PROVIDER_SENTINEL] {
            assert!(!encoded_response.contains(secret));
        }
        let first_enqueue = service
            .enqueue_research(
                &client_id,
                "root",
                "research-2",
                PROMPT_SENTINEL,
                None,
                Some(message_id),
            )
            .await
            .unwrap();
        assert!(
            first_enqueue.inserted,
            "subscription failure must persist nothing"
        );
    }
}
