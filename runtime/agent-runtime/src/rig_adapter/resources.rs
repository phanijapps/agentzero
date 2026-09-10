//! Execution-scoped MCP ownership, independent of stream finalization.

use crate::mcp::McpManager;
use std::sync::Arc;

pub(super) struct SessionResources {
    manager: Option<Arc<McpManager>>,
    runtime: Option<tokio::runtime::Handle>,
}

impl SessionResources {
    pub(super) fn new(manager: Arc<McpManager>) -> Self {
        Self {
            manager: Some(manager),
            runtime: tokio::runtime::Handle::try_current().ok(),
        }
    }

    pub(super) fn for_run(&self) -> Self {
        Self {
            manager: self.manager.clone(),
            runtime: self.runtime.clone(),
        }
    }

    pub(super) async fn close(mut self) {
        if let Some(cleanup) = self.start_cleanup() {
            // Dropping this await detaches the task; it must not interrupt
            // cleanup after the manager has drained its registered clients.
            if cleanup.await.is_err() {
                tracing::warn!("MCP execution cleanup task failed");
            }
        }
    }

    fn start_cleanup(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        let manager = self.manager.take()?;
        let runtime = self
            .runtime
            .clone()
            .or_else(|| tokio::runtime::Handle::try_current().ok())?;
        Some(runtime.spawn(async move { manager.close().await }))
    }
}

impl Drop for SessionResources {
    fn drop(&mut self) {
        // Covers never-run engines and abandoned execution futures. The MCP
        // manager also cancels clients on drop if the runtime has shut down.
        self.start_cleanup();
    }
}
