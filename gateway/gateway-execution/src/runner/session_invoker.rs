//! # Narrow invoker traits
//!
//! Three single-method traits replacing the original three-method
//! `SessionInvoker`. Each handler depends on exactly the trait it needs:
//!
//! - [`SessionSpawner`]    — fresh session (direct API callers).
//! - [`ContinuationSpawner`] — resume an existing root execution
//!   (used by [`super::continuation_watcher::ContinuationWatcher`]).
//! - [`DelegationSpawner`] — spawn a delegated subagent from a
//!   [`DelegationRequest`] (used by [`super::delegation_dispatcher::DelegationDispatcher`]).
//!
//! [`ExecutionRunner`] implements all three so it can be passed wherever
//! any of the traits is required. The companion structs
//! (`RunnerContinuationInvoker`, `RunnerDelegationInvoker`) implement
//! exactly ONE trait each — no more typed-error stubs.

use async_trait::async_trait;
use tokio::sync::OwnedSemaphorePermit;

use crate::config::ExecutionConfig;
use crate::delegation::DelegationRequest;

// ============================================================================
// Narrow traits
// ============================================================================

/// Resume an existing root execution (continuation path).
///
/// The impl loads history, prepends recall, and builds the continuation
/// message itself — callers do NOT supply a message.
#[async_trait]
pub trait ContinuationSpawner: Send + Sync {
    async fn spawn_continuation(
        &self,
        session_id: String,
        root_agent_id: String,
    ) -> Result<(), String>;
}

/// Spawn a delegated subagent. `permit` is the already-acquired global
/// concurrency permit; the impl holds it for the duration of the child
/// execution.
#[async_trait]
pub trait DelegationSpawner: Send + Sync {
    async fn spawn_delegation(
        &self,
        request: DelegationRequest,
        permit: Option<OwnedSemaphorePermit>,
    ) -> Result<(), String>;
}

// ============================================================================
// Test stubs
// ============================================================================

/// Test-only stub that records every call. Implements all three traits so
/// it can be injected into any handler under test without booting the real
/// executor pipeline.
#[cfg(any(test, feature = "test-stubs"))]
use std::sync::Mutex;

#[cfg(any(test, feature = "test-stubs"))]
pub struct StubSessionInvoker {
    pub calls: Mutex<Vec<(ExecutionConfig, String)>>,
    pub continuation_calls: Mutex<Vec<(String, String)>>, // (session_id, root_agent_id)
    pub delegation_calls: Mutex<Vec<DelegationRequest>>,
}

#[cfg(any(test, feature = "test-stubs"))]
impl StubSessionInvoker {
    pub fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            continuation_calls: Mutex::new(Vec::new()),
            delegation_calls: Mutex::new(Vec::new()),
        }
    }
}

#[cfg(any(test, feature = "test-stubs"))]
impl StubSessionInvoker {
    /// Record a session spawn (test recording surface).
    pub async fn spawn_session(
        &self,
        config: ExecutionConfig,
        message: String,
    ) -> Result<(), String> {
        self.calls.lock().unwrap().push((config, message));
        Ok(())
    }
}

#[cfg(any(test, feature = "test-stubs"))]
impl Default for StubSessionInvoker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-stubs"))]
#[async_trait]
impl ContinuationSpawner for StubSessionInvoker {
    async fn spawn_continuation(
        &self,
        session_id: String,
        root_agent_id: String,
    ) -> Result<(), String> {
        self.continuation_calls
            .lock()
            .unwrap()
            .push((session_id, root_agent_id));
        Ok(())
    }
}

#[cfg(any(test, feature = "test-stubs"))]
#[async_trait]
impl DelegationSpawner for StubSessionInvoker {
    async fn spawn_delegation(
        &self,
        request: DelegationRequest,
        _permit: Option<OwnedSemaphorePermit>,
    ) -> Result<(), String> {
        self.delegation_calls.lock().unwrap().push(request);
        Ok(())
    }
}

// ============================================================================
// ExecutionRunner impls (all three traits — it's the "do everything" type)
// ============================================================================

use super::core::ExecutionRunner;

#[async_trait]
impl ContinuationSpawner for ExecutionRunner {
    async fn spawn_continuation(
        &self,
        session_id: String,
        root_agent_id: String,
    ) -> Result<(), String> {
        self.ctx.spawn_continuation(session_id, root_agent_id).await
    }
}

#[async_trait]
impl DelegationSpawner for ExecutionRunner {
    async fn spawn_delegation(
        &self,
        request: DelegationRequest,
        permit: Option<OwnedSemaphorePermit>,
    ) -> Result<(), String> {
        self.ctx.spawn_delegation(request, permit).await
    }
}
