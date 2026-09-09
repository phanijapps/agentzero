//! # Subscription Manager
//!
//! Server-side subscription routing for WebSocket events.
//!
//! This module provides conversation-scoped event routing instead of broadcasting
//! all events to all clients. Clients must explicitly subscribe to conversations
//! they want to receive events for.
//!
//! ## Design
//!
//! - Single `RwLock<SubscriptionState>` protects all state (race-condition free)
//! - Sequence numbers assigned atomically with event routing
//! - Dead clients cleaned up on send failure and via background task
//! - Scoped event filtering: Session scope shows only root events + delegation lifecycle
//! - Metrics for observability

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::debug;

use super::{ServerMessage, SubscriptionScope};

/// Unique client identifier (WebSocket session ID)
pub type ClientId = String;

// =============================================================================
// SCOPE STATE
// =============================================================================

/// State for session-scope filtering.
///
/// Cached root execution IDs for a session allow O(1) filtering without
/// database lookups in the event routing hot path.
#[derive(Debug, Clone, Default)]
pub struct SessionScopeState {
    /// Cached root execution IDs for this session.
    /// A root execution has `parent_execution_id = NULL` and `delegation_type = 'root'`.
    /// Multiple roots possible: one per user turn + continuations after delegations.
    pub root_execution_ids: HashSet<String>,
}

impl SessionScopeState {
    /// Create new scope state with initial root execution IDs.
    pub fn new(root_execution_ids: HashSet<String>) -> Self {
        Self { root_execution_ids }
    }

    /// Check if an execution ID is a known root.
    pub fn is_root(&self, execution_id: &str) -> bool {
        self.root_execution_ids.contains(execution_id)
    }

    /// Add a new root execution ID to the cache.
    pub fn add_root(&mut self, execution_id: String) {
        self.root_execution_ids.insert(execution_id);
    }
}

// =============================================================================
// EVENT FILTERING
// =============================================================================

/// Event metadata for scope filtering decisions.
///
/// This struct carries the minimal information needed to determine
/// whether an event should be sent to a subscriber based on their scope.
#[derive(Debug, Clone)]
pub struct EventMetadata {
    /// Execution ID of the event (if applicable)
    pub execution_id: Option<String>,
    /// Whether this is a delegation lifecycle event (DelegationStarted/Completed)
    pub is_delegation_event: bool,
}

impl EventMetadata {
    /// Create metadata from an execution ID.
    pub fn with_execution(execution_id: impl Into<String>) -> Self {
        Self {
            execution_id: Some(execution_id.into()),
            is_delegation_event: false,
        }
    }

    /// Create metadata for a delegation lifecycle event.
    pub fn delegation() -> Self {
        Self {
            execution_id: None,
            is_delegation_event: true,
        }
    }

    /// Create metadata for session-level events (no execution_id).
    pub fn session_level() -> Self {
        Self {
            execution_id: None,
            is_delegation_event: false,
        }
    }
}

/// Determine if an event should be sent to a subscriber based on their scope.
///
/// Filtering rules:
/// - `All` scope: Send all events
/// - `Session` scope: Send only events from root executions + delegation lifecycle
/// - `Execution(id)` scope: Send only events for that specific execution
pub fn should_send_to_scope(
    metadata: &EventMetadata,
    scope: &SubscriptionScope,
    scope_state: Option<&SessionScopeState>,
) -> bool {
    match scope {
        SubscriptionScope::All => true,
        SubscriptionScope::Session => {
            // Delegation lifecycle events always shown in session view
            if metadata.is_delegation_event {
                return true;
            }

            // Events without execution_id (session-level) always shown
            let Some(ref exec_id) = metadata.execution_id else {
                return true;
            };

            // Check if execution is a root using cached state
            if let Some(state) = scope_state {
                state.is_root(exec_id)
            } else {
                // No scope state = fallback to showing all (backward compatible)
                true
            }
        }
        SubscriptionScope::Execution(target_id) => {
            // Only send events for the specific execution
            metadata.execution_id.as_ref() == Some(target_id)
        }
    }
}

// =============================================================================
// CLIENT STATE
// =============================================================================

/// Client connection state
struct Client {
    id: ClientId,
    sender: mpsc::UnboundedSender<ServerMessage>,
    connected_at: Instant,
    last_activity: Instant,
    subscription_count: usize,
    /// Track if channel has failed (for cleanup)
    channel_healthy: bool,
    surface_catalogs: HashSet<String>,
}

/// Per-subscription state including scope and cached identifiers.
#[derive(Debug, Clone)]
struct SubscriptionEntry {
    /// Client ID
    client_id: ClientId,
    /// Event filtering scope
    scope: SubscriptionScope,
    /// Cached state for Session scope filtering (None for other scopes)
    scope_state: Option<SessionScopeState>,
}

/// All subscription state protected by a SINGLE lock
struct SubscriptionState {
    clients: HashMap<ClientId, Client>,
    /// Map from conversation_id -> set of subscribed client_ids
    subscriptions: HashMap<String, HashSet<ClientId>>,
    /// Map from client_id -> set of subscribed conversation_ids
    client_subscriptions: HashMap<ClientId, HashSet<String>>,
    /// Sequence numbers per conversation
    sequence_numbers: HashMap<String, u64>,
    /// Subscription entries with scope state (conversation_id, client_id) -> entry
    subscription_entries: HashMap<(String, ClientId), SubscriptionEntry>,
    session_owners: HashMap<String, ClientId>,
}

/// Subscription manager for routing events to subscribed clients.
pub struct SubscriptionManager {
    state: RwLock<SubscriptionState>,
    max_subscriptions_per_client: usize,
    max_subscribers_per_conversation: usize,
    metrics: Arc<SubscriptionMetrics>,
}

/// Metrics for observability
#[derive(Default)]
pub struct SubscriptionMetrics {
    pub total_clients: AtomicU64,
    pub total_subscriptions: AtomicU64,
    pub events_routed: AtomicU64,
    pub events_dropped: AtomicU64,
    pub dead_clients_cleaned: AtomicU64,
}

/// Result of event routing
#[derive(Debug)]
pub struct RoutingResult {
    pub sent: u64,
    pub dropped: u64,
    pub dead_clients: Vec<ClientId>,
}

/// Error when subscribing
#[derive(Debug)]
pub enum SubscribeError {
    ClientNotFound,
    TooManySubscriptions { limit: usize },
    ConversationFull { limit: usize },
}

/// Result of successful subscribe
#[derive(Debug)]
pub enum SubscribeResult {
    Subscribed { current_sequence: u64 },
    AlreadySubscribed { current_sequence: u64 },
}

impl SubscriptionManager {
    const DEFAULT_MAX_SUBS_PER_CLIENT: usize = 50;
    const DEFAULT_MAX_SUBS_PER_CONV: usize = 1000;

    /// Create a new subscription manager.
    pub fn new() -> Self {
        Self {
            state: RwLock::new(SubscriptionState {
                clients: HashMap::new(),
                subscriptions: HashMap::new(),
                client_subscriptions: HashMap::new(),
                sequence_numbers: HashMap::new(),
                subscription_entries: HashMap::new(),
                session_owners: HashMap::new(),
            }),
            max_subscriptions_per_client: Self::DEFAULT_MAX_SUBS_PER_CLIENT,
            max_subscribers_per_conversation: Self::DEFAULT_MAX_SUBS_PER_CONV,
            metrics: Arc::new(SubscriptionMetrics::default()),
        }
    }

    /// Get metrics for observability.
    pub fn metrics(&self) -> Arc<SubscriptionMetrics> {
        self.metrics.clone()
    }

    /// Register a new client - atomic operation.
    pub async fn connect(&self, client_id: ClientId, sender: mpsc::UnboundedSender<ServerMessage>) {
        let mut state = self.state.write().await;
        state.clients.insert(
            client_id.clone(),
            Client {
                id: client_id.clone(),
                sender,
                connected_at: Instant::now(),
                last_activity: Instant::now(),
                subscription_count: 0,
                channel_healthy: true,
                surface_catalogs: HashSet::new(),
            },
        );
        state.client_subscriptions.insert(client_id, HashSet::new());
        self.metrics.total_clients.fetch_add(1, Ordering::Relaxed);
    }

    /// Replace a client's explicitly advertised work-surface catalogs.
    pub async fn set_surface_catalogs(&self, client_id: &ClientId, catalogs: HashSet<String>) {
        let mut state = self.state.write().await;
        if let Some(client) = state.clients.get_mut(client_id) {
            client.surface_catalogs = catalogs;
            client.last_activity = Instant::now();
        }
    }

    /// Update client's last activity timestamp to prevent stale cleanup.
    /// Call this on any client interaction (subscribe, unsubscribe, ping, etc.)
    pub async fn touch_client(&self, client_id: &ClientId) {
        let mut state = self.state.write().await;
        if let Some(client) = state.clients.get_mut(client_id) {
            client.last_activity = Instant::now();
        }
    }

    /// Bind only server-accepted invokes to their originating connection.
    pub async fn bind_session_owner(&self, client_id: &ClientId, session_id: String) {
        let mut state = self.state.write().await;
        if state.clients.contains_key(client_id) {
            state.session_owners.insert(session_id, client_id.clone());
        }
    }

    pub async fn owns_session(&self, client_id: &ClientId, session_id: &str) -> bool {
        self.state.read().await.session_owners.get(session_id) == Some(client_id)
    }

    /// Disconnect and cleanup - atomic, no race conditions.
    pub async fn disconnect(&self, client_id: &ClientId) {
        let mut state = self.state.write().await;
        self.disconnect_internal(&mut state, client_id);
    }

    /// Internal disconnect with state already locked.
    fn disconnect_internal(&self, state: &mut SubscriptionState, client_id: &ClientId) {
        state.session_owners.retain(|_, owner| owner != client_id);
        if let Some(conversations) = state.client_subscriptions.remove(client_id) {
            for conv_id in &conversations {
                if let Some(subscribers) = state.subscriptions.get_mut(conv_id) {
                    subscribers.remove(client_id);
                    if subscribers.is_empty() {
                        state.subscriptions.remove(conv_id);
                        // Also clean up sequence numbers for empty conversations
                        state.sequence_numbers.remove(conv_id);
                    }
                }
                // Clean up subscription entry
                let entry_key = (conv_id.clone(), client_id.clone());
                state.subscription_entries.remove(&entry_key);
            }
            self.metrics
                .total_subscriptions
                .fetch_sub(conversations.len() as u64, Ordering::Relaxed);
        }
        state.clients.remove(client_id);
        self.metrics.total_clients.fetch_sub(1, Ordering::Relaxed);
    }

    /// Subscribe to a conversation - atomic with limit checks.
    ///
    /// The `scope` parameter controls event filtering:
    /// - `All`: All events for the conversation (default, backward compatible)
    /// - `Session`: Root execution events + delegation lifecycle only
    /// - `Execution(id)`: All events for a specific execution
    ///
    /// For `Session` scope, pass `scope_state` with cached root execution IDs.
    pub async fn subscribe(
        &self,
        client_id: &ClientId,
        conversation_id: String,
    ) -> Result<SubscribeResult, SubscribeError> {
        // Backward-compatible: default to All scope with no scope state
        self.subscribe_with_scope(client_id, conversation_id, SubscriptionScope::All, None)
            .await
    }

    /// Subscribe to a conversation with explicit scope and state.
    ///
    /// For `Session` scope, `scope_state` should contain cached root execution IDs.
    pub async fn subscribe_with_scope(
        &self,
        client_id: &ClientId,
        conversation_id: String,
        scope: SubscriptionScope,
        scope_state: Option<SessionScopeState>,
    ) -> Result<SubscribeResult, SubscribeError> {
        let mut state = self.state.write().await;

        // Check client exists and get subscription count
        let subscription_count = state
            .clients
            .get(client_id)
            .map(|c| c.subscription_count)
            .ok_or(SubscribeError::ClientNotFound)?;

        if subscription_count >= self.max_subscriptions_per_client {
            return Err(SubscribeError::TooManySubscriptions {
                limit: self.max_subscriptions_per_client,
            });
        }

        // Check if already subscribed
        let already_subscribed = state
            .subscriptions
            .get(&conversation_id)
            .map(|s| s.contains(client_id))
            .unwrap_or(false);

        if already_subscribed {
            // Update scope if it changed (e.g., switching from Session to Execution scope)
            let entry_key = (conversation_id.clone(), client_id.clone());
            if let Some(entry) = state.subscription_entries.get_mut(&entry_key) {
                if entry.scope != scope {
                    debug!(
                        "Updating subscription scope for {} from {:?} to {:?}",
                        conversation_id, entry.scope, scope
                    );
                    entry.scope = scope;
                    entry.scope_state = scope_state;
                }
            }
            let current_seq = *state.sequence_numbers.get(&conversation_id).unwrap_or(&0);
            return Ok(SubscribeResult::AlreadySubscribed {
                current_sequence: current_seq,
            });
        }

        // Check conversation subscriber limit
        let subscriber_count = state
            .subscriptions
            .get(&conversation_id)
            .map(|s| s.len())
            .unwrap_or(0);

        if subscriber_count >= self.max_subscribers_per_conversation {
            return Err(SubscribeError::ConversationFull {
                limit: self.max_subscribers_per_conversation,
            });
        }

        // All checks passed, now do the updates
        state
            .subscriptions
            .entry(conversation_id.clone())
            .or_insert_with(HashSet::new)
            .insert(client_id.clone());

        state
            .client_subscriptions
            .get_mut(client_id)
            .unwrap()
            .insert(conversation_id.clone());

        if let Some(client) = state.clients.get_mut(client_id) {
            client.subscription_count += 1;
        }

        // Store subscription entry with scope and state
        let entry_key = (conversation_id.clone(), client_id.clone());
        state.subscription_entries.insert(
            entry_key,
            SubscriptionEntry {
                client_id: client_id.clone(),
                scope,
                scope_state,
            },
        );

        let current_seq = *state.sequence_numbers.entry(conversation_id).or_insert(0);

        self.metrics
            .total_subscriptions
            .fetch_add(1, Ordering::Relaxed);

        Ok(SubscribeResult::Subscribed {
            current_sequence: current_seq,
        })
    }

    /// Unsubscribe from a conversation - atomic.
    pub async fn unsubscribe(&self, client_id: &ClientId, conversation_id: &str) {
        let mut state = self.state.write().await;

        // Check if client was actually subscribed
        let was_subscribed = state
            .subscriptions
            .get_mut(conversation_id)
            .map(|s| s.remove(client_id))
            .unwrap_or(false);

        if !was_subscribed {
            return;
        }

        // Remove subscription entry
        let entry_key = (conversation_id.to_string(), client_id.clone());
        state.subscription_entries.remove(&entry_key);

        self.metrics
            .total_subscriptions
            .fetch_sub(1, Ordering::Relaxed);

        // Update client subscription count
        if let Some(client) = state.clients.get_mut(client_id) {
            client.subscription_count = client.subscription_count.saturating_sub(1);
        }

        // Check if conversation is now empty
        let is_empty = state
            .subscriptions
            .get(conversation_id)
            .map(|s| s.is_empty())
            .unwrap_or(false);

        if is_empty {
            state.subscriptions.remove(conversation_id);
            state.sequence_numbers.remove(conversation_id);
        }

        // Remove from client's subscription list
        if let Some(client_subs) = state.client_subscriptions.get_mut(client_id) {
            client_subs.remove(conversation_id);
        }
    }

    /// Route event to subscribed clients with atomic sequence assignment.
    ///
    /// Returns routing result with counts and dead clients.
    pub async fn route_event(
        &self,
        conversation_id: &str,
        message: ServerMessage,
    ) -> RoutingResult {
        let mut state = self.state.write().await;

        // Assign sequence number atomically
        let seq = state
            .sequence_numbers
            .entry(conversation_id.to_string())
            .or_insert(0);
        *seq += 1;
        let current_seq = *seq;

        // Add sequence to message
        let message_with_seq = message.with_sequence(current_seq);

        // Collect subscriber IDs first to avoid borrowing conflicts
        let subscriber_ids: Vec<ClientId> = state
            .subscriptions
            .get(conversation_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();

        if subscriber_ids.is_empty() {
            return RoutingResult {
                sent: 0,
                dropped: 0,
                dead_clients: vec![],
            };
        }

        let mut sent = 0u64;
        let mut dropped = 0u64;
        let mut dead_clients = Vec::new();

        for client_id in subscriber_ids {
            if let Some(client) = state.clients.get_mut(&client_id) {
                match client.sender.send(message_with_seq.clone()) {
                    Ok(()) => {
                        sent += 1;
                        client.last_activity = Instant::now();
                    }
                    Err(_) => {
                        dropped += 1;
                        // Channel closed - mark for cleanup
                        client.channel_healthy = false;
                        dead_clients.push(client_id);
                    }
                }
            }
        }

        // Clean up dead clients while we still hold the lock
        for dead_id in &dead_clients {
            self.disconnect_internal(&mut state, dead_id);
            self.metrics
                .dead_clients_cleaned
                .fetch_add(1, Ordering::Relaxed);
        }

        self.metrics
            .events_routed
            .fetch_add(sent, Ordering::Relaxed);
        self.metrics
            .events_dropped
            .fetch_add(dropped, Ordering::Relaxed);

        RoutingResult {
            sent,
            dropped,
            dead_clients,
        }
    }

    /// Route event to subscribed clients with scope-based filtering.
    ///
    /// This method applies the subscription scope filter before sending:
    /// - `All` scope: Receives all events
    /// - `Session` scope: Receives only root execution events + delegation lifecycle
    /// - `Execution(id)` scope: Receives only events for that execution
    ///
    /// Returns routing result with counts and dead clients.
    pub async fn route_event_scoped(
        &self,
        conversation_id: &str,
        message: ServerMessage,
        metadata: &EventMetadata,
    ) -> RoutingResult {
        let mut state = self.state.write().await;

        // Assign sequence number atomically
        let seq = state
            .sequence_numbers
            .entry(conversation_id.to_string())
            .or_insert(0);
        *seq += 1;
        let current_seq = *seq;

        // Add sequence to message
        let message_with_seq = message.with_sequence(current_seq);

        // Collect subscriber IDs first to avoid borrowing conflicts
        let subscriber_ids: Vec<ClientId> = state
            .subscriptions
            .get(conversation_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();

        if subscriber_ids.is_empty() {
            return RoutingResult {
                sent: 0,
                dropped: 0,
                dead_clients: vec![],
            };
        }

        let mut sent = 0u64;
        let mut dropped = 0u64;
        let mut filtered = 0u64;
        let mut dead_clients = Vec::new();

        for client_id in subscriber_ids {
            // Get scope info for this subscription
            let entry_key = (conversation_id.to_string(), client_id.clone());
            let (scope, scope_state) = state
                .subscription_entries
                .get(&entry_key)
                .map(|e| (e.scope.clone(), e.scope_state.clone()))
                .unwrap_or((SubscriptionScope::All, None));

            // Apply scope filter
            if !should_send_to_scope(metadata, &scope, scope_state.as_ref()) {
                filtered += 1;
                tracing::warn!(
                    client_id = %client_id,
                    conversation_id = %conversation_id,
                    scope = ?scope,
                    execution_id = ?metadata.execution_id,
                    scope_state_roots = ?scope_state.as_ref().map(|s| &s.root_execution_ids),
                    "Filtered event due to scope"
                );
                continue;
            }

            if matches!(
                message_with_seq,
                ServerMessage::SurfaceCreated { .. }
                    | ServerMessage::SurfaceUpdated { .. }
                    | ServerMessage::SurfaceDeleted { .. }
                    | ServerMessage::SurfaceValidationFailed { .. }
            ) && !state
                .clients
                .get(&client_id)
                .is_some_and(|client| client.surface_catalogs.contains("zbot/work-surface/v1"))
            {
                filtered += 1;
                continue;
            }

            // Send to client
            if let Some(client) = state.clients.get_mut(&client_id) {
                match client.sender.send(message_with_seq.clone()) {
                    Ok(()) => {
                        sent += 1;
                        client.last_activity = Instant::now();
                    }
                    Err(_) => {
                        dropped += 1;
                        // Channel closed - mark for cleanup
                        client.channel_healthy = false;
                        dead_clients.push(client_id);
                    }
                }
            }
        }

        // Clean up dead clients while we still hold the lock
        for dead_id in &dead_clients {
            self.disconnect_internal(&mut state, dead_id);
            self.metrics
                .dead_clients_cleaned
                .fetch_add(1, Ordering::Relaxed);
        }

        // Log filtering stats if any were filtered
        if filtered > 0 {
            debug!(
                conversation_id = %conversation_id,
                sent = sent,
                filtered = filtered,
                "Scoped event routing complete"
            );
        }

        self.metrics
            .events_routed
            .fetch_add(sent, Ordering::Relaxed);
        self.metrics
            .events_dropped
            .fetch_add(dropped, Ordering::Relaxed);

        RoutingResult {
            sent,
            dropped,
            dead_clients,
        }
    }

    /// Broadcast global event to all clients (for stats, notifications).
    pub async fn broadcast_global(&self, message: ServerMessage) {
        let state = self.state.read().await;

        for client in state.clients.values() {
            if client.channel_healthy {
                let _ = client.sender.send(message.clone());
            }
        }
    }

    /// Send message to a specific client.
    pub async fn send_to_client(&self, client_id: &ClientId, message: ServerMessage) -> bool {
        let state = self.state.read().await;

        if let Some(client) = state.clients.get(client_id) {
            if client.channel_healthy {
                return client.sender.send(message).is_ok();
            }
        }
        false
    }

    /// Background cleanup task - call periodically (e.g., every 30s).
    pub async fn cleanup_stale_clients(&self, timeout: Duration) -> usize {
        let mut state = self.state.write().await;
        let now = Instant::now();

        let stale: Vec<ClientId> = state
            .clients
            .iter()
            .filter(|(_, c)| !c.channel_healthy || now.duration_since(c.last_activity) > timeout)
            .map(|(id, _)| id.clone())
            .collect();

        let count = stale.len();
        for client_id in stale {
            debug!(client_id = %client_id, "Cleaning up stale client");
            self.disconnect_internal(&mut state, &client_id);
        }

        self.metrics
            .dead_clients_cleaned
            .fetch_add(count as u64, Ordering::Relaxed);
        count
    }

    /// Get the number of connected clients.
    pub async fn client_count(&self) -> usize {
        self.state.read().await.clients.len()
    }

    /// Get the number of active subscriptions.
    pub async fn subscription_count(&self) -> usize {
        self.state
            .read()
            .await
            .subscriptions
            .values()
            .map(|s| s.len())
            .sum()
    }

    /// Check if a client is subscribed to a conversation.
    pub async fn is_subscribed(&self, client_id: &ClientId, conversation_id: &str) -> bool {
        let state = self.state.read().await;
        state
            .subscriptions
            .get(conversation_id)
            .map(|s| s.contains(client_id))
            .unwrap_or(false)
    }

    // =========================================================================
    // SCOPE MANAGEMENT
    // =========================================================================

    /// Add a root execution ID to caches of all session-scoped subscribers for a conversation.
    ///
    /// Called when a new root execution starts (AgentStarted with parent_execution_id = null).
    /// This ensures the cache stays current for multi-turn sessions and continuations.
    pub async fn add_root_to_caches(&self, conversation_id: &str, execution_id: &str) {
        let mut state = self.state.write().await;

        // Find all subscription entries for this conversation
        let subscriber_ids: Vec<ClientId> = state
            .subscriptions
            .get(conversation_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();

        tracing::info!(
            conversation_id = %conversation_id,
            execution_id = %execution_id,
            subscriber_count = subscriber_ids.len(),
            "add_root_to_caches called"
        );

        // Update scope state for each session-scoped subscriber
        for client_id in subscriber_ids {
            let entry_key = (conversation_id.to_string(), client_id);
            if let Some(entry) = state.subscription_entries.get_mut(&entry_key) {
                if matches!(entry.scope, SubscriptionScope::Session) {
                    if let Some(ref mut scope_state) = entry.scope_state {
                        scope_state.add_root(execution_id.to_string());
                        debug!(
                            conversation_id = %conversation_id,
                            execution_id = %execution_id,
                            "Added root execution to session scope cache"
                        );
                    }
                }
            }
        }
    }

    /// Get the subscription scope and state for a specific client subscription.
    ///
    /// Returns `None` if the client is not subscribed to the conversation.
    pub async fn get_subscription_scope(
        &self,
        client_id: &ClientId,
        conversation_id: &str,
    ) -> Option<(SubscriptionScope, Option<SessionScopeState>)> {
        let state = self.state.read().await;
        let entry_key = (conversation_id.to_string(), client_id.clone());
        state
            .subscription_entries
            .get(&entry_key)
            .map(|entry| (entry.scope.clone(), entry.scope_state.clone()))
    }

    /// Get all subscription entries for a conversation (for scoped routing).
    ///
    /// Returns a list of (client_id, scope, scope_state) tuples for all subscribers.
    pub async fn get_subscription_entries(
        &self,
        conversation_id: &str,
    ) -> Vec<(ClientId, SubscriptionScope, Option<SessionScopeState>)> {
        let state = self.state.read().await;

        let subscriber_ids: Vec<ClientId> = state
            .subscriptions
            .get(conversation_id)
            .map(|s| s.iter().cloned().collect())
            .unwrap_or_default();

        subscriber_ids
            .into_iter()
            .filter_map(|client_id| {
                let entry_key = (conversation_id.to_string(), client_id.clone());
                state
                    .subscription_entries
                    .get(&entry_key)
                    .map(|entry| (client_id, entry.scope.clone(), entry.scope_state.clone()))
            })
            .collect()
    }
}

impl Default for SubscriptionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_sender() -> (
        mpsc::UnboundedSender<ServerMessage>,
        mpsc::UnboundedReceiver<ServerMessage>,
    ) {
        mpsc::unbounded_channel()
    }

    #[tokio::test]
    async fn test_connect_disconnect() {
        let manager = SubscriptionManager::new();
        let (tx, _rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;
        assert_eq!(manager.client_count().await, 1);

        manager.disconnect(&"client-1".to_string()).await;
        assert_eq!(manager.client_count().await, 0);
    }

    #[tokio::test]
    async fn test_subscribe_unsubscribe() {
        let manager = SubscriptionManager::new();
        let (tx, _rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;

        // Subscribe
        let result = manager
            .subscribe(&"client-1".to_string(), "conv-1".to_string())
            .await;
        assert!(matches!(
            result,
            Ok(SubscribeResult::Subscribed {
                current_sequence: 0
            })
        ));
        assert!(
            manager
                .is_subscribed(&"client-1".to_string(), "conv-1")
                .await
        );

        // Double subscribe returns AlreadySubscribed
        let result = manager
            .subscribe(&"client-1".to_string(), "conv-1".to_string())
            .await;
        assert!(matches!(
            result,
            Ok(SubscribeResult::AlreadySubscribed { .. })
        ));

        // Unsubscribe
        manager.unsubscribe(&"client-1".to_string(), "conv-1").await;
        assert!(
            !manager
                .is_subscribed(&"client-1".to_string(), "conv-1")
                .await
        );
    }

    #[tokio::test]
    async fn test_subscribe_limit() {
        let manager = SubscriptionManager::new();
        let (tx, _rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;

        // Subscribe up to limit
        for i in 0..SubscriptionManager::DEFAULT_MAX_SUBS_PER_CLIENT {
            let result = manager
                .subscribe(&"client-1".to_string(), format!("conv-{}", i))
                .await;
            assert!(result.is_ok());
        }

        // Next subscription should fail
        let result = manager
            .subscribe(&"client-1".to_string(), "conv-overflow".to_string())
            .await;
        assert!(matches!(
            result,
            Err(SubscribeError::TooManySubscriptions { .. })
        ));
    }

    #[tokio::test]
    async fn test_route_event() {
        let manager = SubscriptionManager::new();
        let (tx, mut rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;
        manager
            .subscribe(&"client-1".to_string(), "conv-1".to_string())
            .await
            .unwrap();

        // Route event
        let msg = ServerMessage::Token {
            session_id: "sess-1".to_string(),
            execution_id: "exec-1".to_string(),
            conversation_id: Some("conv-1".to_string()),
            delta: "Hello".to_string(),
            seq: None,
        };
        let result = manager.route_event("conv-1", msg).await;

        assert_eq!(result.sent, 1);
        assert_eq!(result.dropped, 0);
        assert!(result.dead_clients.is_empty());

        // Check received message has sequence
        let received = rx.recv().await.unwrap();
        if let ServerMessage::Token { seq, .. } = received {
            assert_eq!(seq, Some(1));
        } else {
            panic!("Wrong message type");
        }
    }

    #[tokio::test]
    async fn test_route_event_sequence_increments() {
        let manager = SubscriptionManager::new();
        let (tx, mut rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;
        manager
            .subscribe(&"client-1".to_string(), "conv-1".to_string())
            .await
            .unwrap();

        // Route multiple events
        for _ in 0..3 {
            let msg = ServerMessage::Token {
                session_id: "sess-1".to_string(),
                execution_id: "exec-1".to_string(),
                conversation_id: Some("conv-1".to_string()),
                delta: "x".to_string(),
                seq: None,
            };
            manager.route_event("conv-1", msg).await;
        }

        // Check sequences
        for expected_seq in 1..=3 {
            let received = rx.recv().await.unwrap();
            if let ServerMessage::Token { seq, .. } = received {
                assert_eq!(seq, Some(expected_seq));
            }
        }
    }

    #[tokio::test]
    async fn test_dead_client_cleanup() {
        let manager = SubscriptionManager::new();
        let (tx, rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;
        manager
            .subscribe(&"client-1".to_string(), "conv-1".to_string())
            .await
            .unwrap();

        // Drop receiver to simulate dead client
        drop(rx);

        // Route event - should detect dead client
        let msg = ServerMessage::Token {
            session_id: "sess-1".to_string(),
            execution_id: "exec-1".to_string(),
            conversation_id: Some("conv-1".to_string()),
            delta: "Hello".to_string(),
            seq: None,
        };
        let result = manager.route_event("conv-1", msg).await;

        assert_eq!(result.sent, 0);
        assert_eq!(result.dropped, 1);
        assert_eq!(result.dead_clients.len(), 1);

        // Client should be cleaned up
        assert_eq!(manager.client_count().await, 0);
    }

    #[tokio::test]
    async fn test_disconnect_cleans_subscriptions() {
        let manager = SubscriptionManager::new();
        let (tx, _rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;
        manager
            .subscribe(&"client-1".to_string(), "conv-1".to_string())
            .await
            .unwrap();
        manager
            .subscribe(&"client-1".to_string(), "conv-2".to_string())
            .await
            .unwrap();

        assert_eq!(manager.subscription_count().await, 2);

        // Disconnect should clean up all subscriptions
        manager.disconnect(&"client-1".to_string()).await;

        assert_eq!(manager.subscription_count().await, 0);
        assert_eq!(manager.client_count().await, 0);
    }

    #[tokio::test]
    async fn test_broadcast_global() {
        let manager = SubscriptionManager::new();
        let (tx1, mut rx1) = create_test_sender();
        let (tx2, mut rx2) = create_test_sender();

        manager.connect("client-1".to_string(), tx1).await;
        manager.connect("client-2".to_string(), tx2).await;

        // Broadcast (no subscription needed)
        let msg = ServerMessage::Pong;
        manager.broadcast_global(msg).await;

        // Both clients should receive
        assert!(rx1.recv().await.is_some());
        assert!(rx2.recv().await.is_some());
    }

    #[tokio::test]
    async fn test_conversation_isolation() {
        let manager = SubscriptionManager::new();
        let (tx1, mut rx1) = create_test_sender();
        let (tx2, mut rx2) = create_test_sender();

        manager.connect("client-1".to_string(), tx1).await;
        manager.connect("client-2".to_string(), tx2).await;

        // Client 1 subscribes to conv-1
        manager
            .subscribe(&"client-1".to_string(), "conv-1".to_string())
            .await
            .unwrap();

        // Client 2 subscribes to conv-2
        manager
            .subscribe(&"client-2".to_string(), "conv-2".to_string())
            .await
            .unwrap();

        // Route event to conv-1
        let msg = ServerMessage::Token {
            session_id: "sess-1".to_string(),
            execution_id: "exec-1".to_string(),
            conversation_id: Some("conv-1".to_string()),
            delta: "Hello".to_string(),
            seq: None,
        };
        manager.route_event("conv-1", msg).await;

        // Only client-1 should receive
        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_err());
    }

    // =========================================================================
    // SCOPE FILTERING TESTS
    // =========================================================================

    #[test]
    fn test_should_send_to_scope_all_always_returns_true() {
        let metadata = EventMetadata {
            execution_id: Some("exec-subagent".to_string()),
            is_delegation_event: false,
        };
        let scope = SubscriptionScope::All;

        assert!(should_send_to_scope(&metadata, &scope, None));
    }

    #[test]
    fn test_should_send_to_scope_session_root_execution() {
        let mut roots = HashSet::new();
        roots.insert("exec-root".to_string());
        let scope_state = SessionScopeState::new(roots);

        let metadata = EventMetadata {
            execution_id: Some("exec-root".to_string()),
            is_delegation_event: false,
        };
        let scope = SubscriptionScope::Session;

        assert!(should_send_to_scope(&metadata, &scope, Some(&scope_state)));
    }

    #[test]
    fn test_should_send_to_scope_session_non_root_filtered() {
        let mut roots = HashSet::new();
        roots.insert("exec-root".to_string());
        let scope_state = SessionScopeState::new(roots);

        let metadata = EventMetadata {
            execution_id: Some("exec-subagent".to_string()),
            is_delegation_event: false,
        };
        let scope = SubscriptionScope::Session;

        assert!(!should_send_to_scope(&metadata, &scope, Some(&scope_state)));
    }

    #[test]
    fn test_should_send_to_scope_session_delegation_always_passes() {
        let scope_state = SessionScopeState::new(HashSet::new()); // Empty roots

        let metadata = EventMetadata {
            execution_id: Some("exec-subagent".to_string()),
            is_delegation_event: true, // Delegation event
        };
        let scope = SubscriptionScope::Session;

        // Delegation events always pass through session scope
        assert!(should_send_to_scope(&metadata, &scope, Some(&scope_state)));
    }

    #[test]
    fn test_should_send_to_scope_session_no_execution_id_passes() {
        let scope_state = SessionScopeState::new(HashSet::new());

        let metadata = EventMetadata {
            execution_id: None, // Session-level event
            is_delegation_event: false,
        };
        let scope = SubscriptionScope::Session;

        // Events without execution_id (session-level) always pass
        assert!(should_send_to_scope(&metadata, &scope, Some(&scope_state)));
    }

    #[test]
    fn test_should_send_to_scope_execution_matching() {
        let metadata = EventMetadata {
            execution_id: Some("exec-123".to_string()),
            is_delegation_event: false,
        };
        let scope = SubscriptionScope::Execution("exec-123".to_string());

        assert!(should_send_to_scope(&metadata, &scope, None));
    }

    #[test]
    fn test_should_send_to_scope_execution_non_matching() {
        let metadata = EventMetadata {
            execution_id: Some("exec-456".to_string()),
            is_delegation_event: false,
        };
        let scope = SubscriptionScope::Execution("exec-123".to_string());

        assert!(!should_send_to_scope(&metadata, &scope, None));
    }

    #[tokio::test]
    async fn test_subscribe_with_session_scope() {
        let manager = SubscriptionManager::new();
        let (tx, _rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;

        let mut root_ids = HashSet::new();
        root_ids.insert("exec-root-1".to_string());
        root_ids.insert("exec-root-2".to_string());
        let scope_state = SessionScopeState::new(root_ids);

        let result = manager
            .subscribe_with_scope(
                &"client-1".to_string(),
                "sess-1".to_string(),
                SubscriptionScope::Session,
                Some(scope_state),
            )
            .await;

        assert!(matches!(result, Ok(SubscribeResult::Subscribed { .. })));

        // Verify scope was stored
        let (scope, state) = manager
            .get_subscription_scope(&"client-1".to_string(), "sess-1")
            .await
            .unwrap();

        assert!(matches!(scope, SubscriptionScope::Session));
        assert!(state.is_some());
        let state = state.unwrap();
        assert!(state.is_root("exec-root-1"));
        assert!(state.is_root("exec-root-2"));
        assert!(!state.is_root("exec-subagent"));
    }

    #[tokio::test]
    async fn test_route_event_scoped_filters_non_root() {
        let manager = SubscriptionManager::new();
        let (tx, mut rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;

        // Subscribe with session scope - only exec-root is root
        let mut root_ids = HashSet::new();
        root_ids.insert("exec-root".to_string());
        let scope_state = SessionScopeState::new(root_ids);

        manager
            .subscribe_with_scope(
                &"client-1".to_string(),
                "sess-1".to_string(),
                SubscriptionScope::Session,
                Some(scope_state),
            )
            .await
            .unwrap();

        // Send event from subagent - should be filtered
        let subagent_msg = ServerMessage::Token {
            session_id: "sess-1".to_string(),
            execution_id: "exec-subagent".to_string(),
            conversation_id: None,
            delta: "subagent output".to_string(),
            seq: None,
        };
        let metadata = EventMetadata {
            execution_id: Some("exec-subagent".to_string()),
            is_delegation_event: false,
        };
        let result = manager
            .route_event_scoped("sess-1", subagent_msg, &metadata)
            .await;

        // Event was filtered, nothing sent
        assert_eq!(result.sent, 0);
        assert!(rx.try_recv().is_err());

        // Send event from root - should pass
        let root_msg = ServerMessage::Token {
            session_id: "sess-1".to_string(),
            execution_id: "exec-root".to_string(),
            conversation_id: None,
            delta: "root output".to_string(),
            seq: None,
        };
        let root_metadata = EventMetadata {
            execution_id: Some("exec-root".to_string()),
            is_delegation_event: false,
        };
        let result = manager
            .route_event_scoped("sess-1", root_msg, &root_metadata)
            .await;

        assert_eq!(result.sent, 1);
        let received = rx.recv().await.unwrap();
        if let ServerMessage::Token { delta, .. } = received {
            assert_eq!(delta, "root output");
        } else {
            panic!("Expected Token message");
        }
    }

    #[tokio::test]
    async fn test_add_root_to_caches() {
        let manager = SubscriptionManager::new();
        let (tx, _rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;

        // Subscribe with session scope - initially only exec-root-1
        let mut root_ids = HashSet::new();
        root_ids.insert("exec-root-1".to_string());
        let scope_state = SessionScopeState::new(root_ids);

        manager
            .subscribe_with_scope(
                &"client-1".to_string(),
                "sess-1".to_string(),
                SubscriptionScope::Session,
                Some(scope_state),
            )
            .await
            .unwrap();

        // Add new root (continuation)
        manager.add_root_to_caches("sess-1", "exec-root-2").await;

        // Verify new root is in cache
        let (_, state) = manager
            .get_subscription_scope(&"client-1".to_string(), "sess-1")
            .await
            .unwrap();

        let state = state.unwrap();
        assert!(state.is_root("exec-root-1"));
        assert!(state.is_root("exec-root-2"));
    }

    #[tokio::test]
    async fn test_route_event_scoped_delegation_events_pass_through() {
        let manager = SubscriptionManager::new();
        let (tx, mut rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;

        // Subscribe with session scope with empty roots
        manager
            .subscribe_with_scope(
                &"client-1".to_string(),
                "sess-1".to_string(),
                SubscriptionScope::Session,
                Some(SessionScopeState::new(HashSet::new())),
            )
            .await
            .unwrap();

        // Send delegation event - should pass even though roots is empty
        let msg = ServerMessage::DelegationStarted {
            session_id: "sess-1".to_string(),
            parent_execution_id: "exec-root".to_string(),
            child_execution_id: "exec-child".to_string(),
            parent_agent_id: "root-agent".to_string(),
            child_agent_id: "child-agent".to_string(),
            task: "Do something".to_string(),
            parent_conversation_id: None,
            child_conversation_id: None,
            seq: None,
        };
        let metadata = EventMetadata {
            execution_id: Some("exec-root".to_string()),
            is_delegation_event: true, // Key: delegation events always pass
        };
        let result = manager.route_event_scoped("sess-1", msg, &metadata).await;

        assert_eq!(result.sent, 1);
        let received = rx.recv().await.unwrap();
        assert!(matches!(received, ServerMessage::DelegationStarted { .. }));
    }

    #[tokio::test]
    async fn test_subscribe_scope_update_on_resubscribe() {
        // Tests that re-subscribing with a different scope updates the filter
        // This is critical for: Session scope -> Execution scope transition
        // (e.g., viewing subagent detail from main chat view)
        let manager = SubscriptionManager::new();
        let (tx, mut rx) = create_test_sender();

        manager.connect("client-1".to_string(), tx).await;

        // First subscribe with Session scope
        let mut root_ids = HashSet::new();
        root_ids.insert("exec-root".to_string());
        let scope_state = SessionScopeState::new(root_ids);

        manager
            .subscribe_with_scope(
                &"client-1".to_string(),
                "sess-1".to_string(),
                SubscriptionScope::Session,
                Some(scope_state),
            )
            .await
            .unwrap();

        // Verify Session scope filters out subagent events
        let subagent_msg = ServerMessage::Token {
            session_id: "sess-1".to_string(),
            execution_id: "exec-subagent".to_string(),
            conversation_id: None,
            delta: "subagent output".to_string(),
            seq: None,
        };
        let metadata = EventMetadata::with_execution("exec-subagent");
        let result = manager
            .route_event_scoped("sess-1", subagent_msg.clone(), &metadata)
            .await;
        assert_eq!(
            result.sent, 0,
            "Session scope should filter subagent events"
        );

        // Now re-subscribe with Execution scope targeting the subagent
        let result = manager
            .subscribe_with_scope(
                &"client-1".to_string(),
                "sess-1".to_string(),
                SubscriptionScope::Execution("exec-subagent".to_string()),
                None,
            )
            .await;

        // Should return AlreadySubscribed (but scope was updated internally)
        assert!(matches!(
            result,
            Ok(SubscribeResult::AlreadySubscribed { .. })
        ));

        // Verify scope was updated - subagent events should now pass
        let result = manager
            .route_event_scoped("sess-1", subagent_msg, &metadata)
            .await;
        assert_eq!(
            result.sent, 1,
            "Execution scope should allow subagent events"
        );

        // Verify we received the event
        let received = rx.recv().await.unwrap();
        if let ServerMessage::Token { delta, .. } = received {
            assert_eq!(delta, "subagent output");
        } else {
            panic!("Expected Token message");
        }
    }
}
