//! # Execution Handle
//!
//! Handle for controlling running agent executions.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

/// Handle to a running execution, allowing control operations.
///
/// This handle provides thread-safe control over an agent execution,
/// allowing external code to stop, pause, resume, or cancel the execution.
#[derive(Clone)]
pub struct ExecutionHandle {
    /// Flag to signal stop
    stop_flag: Arc<AtomicBool>,
    /// Flag to signal pause
    pause_flag: Arc<AtomicBool>,
    /// Flag to signal cancel
    cancel_flag: Arc<AtomicBool>,
    /// Current iteration counter
    iteration: Arc<AtomicU32>,
    /// Maximum iterations
    max_iterations: Arc<AtomicU32>,
}

impl ExecutionHandle {
    /// Create a new execution handle with the specified max iterations.
    pub fn new(max_iterations: u32) -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            pause_flag: Arc::new(AtomicBool::new(false)),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            iteration: Arc::new(AtomicU32::new(0)),
            max_iterations: Arc::new(AtomicU32::new(max_iterations)),
        }
    }

    /// Whether two handle values refer to the same live execution. Used by
    /// setup-failure cleanup so an older failed invocation cannot remove a
    /// handle that a newer invocation installed for the same conversation.
    pub(crate) fn is_same_execution(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.stop_flag, &other.stop_flag)
    }

    /// Request the execution to stop.
    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    /// Check if stop was requested.
    pub fn is_stop_requested(&self) -> bool {
        self.stop_flag.load(Ordering::SeqCst)
    }

    /// Get a cloned `Arc` to the underlying stop flag.
    ///
    /// Exposed so the agent-runtime executor can poll it inside its
    /// streaming-LLM `select!` loop without depending on the
    /// `gateway-execution` crate. Plumbing this through `execute_stream`
    /// lets the executor abort an in-flight LLM stream on stop instead
    /// of waiting for the call to finish naturally.
    pub fn stop_signal(&self) -> Arc<AtomicBool> {
        self.stop_flag.clone()
    }

    /// Request the execution to pause.
    pub fn pause(&self) {
        self.pause_flag.store(true, Ordering::SeqCst);
    }

    /// Resume a paused execution.
    pub fn resume(&self) {
        self.pause_flag.store(false, Ordering::SeqCst);
    }

    /// Check if pause was requested.
    pub fn is_paused(&self) -> bool {
        self.pause_flag.load(Ordering::SeqCst)
    }

    /// Request the execution to cancel.
    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
        // Also set stop flag so execution stops immediately
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    /// Check if cancel was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancel_flag.load(Ordering::SeqCst)
    }

    /// Get current iteration.
    pub fn current_iteration(&self) -> u32 {
        self.iteration.load(Ordering::SeqCst)
    }

    /// Increment iteration counter.
    pub fn increment(&self) {
        self.iteration.fetch_add(1, Ordering::SeqCst);
    }

    /// Add more iterations for continuation.
    pub fn add_iterations(&self, additional: u32) {
        self.max_iterations.fetch_add(additional, Ordering::SeqCst);
        self.stop_flag.store(false, Ordering::SeqCst);
    }

    /// Get max iterations.
    pub fn max_iterations(&self) -> u32 {
        self.max_iterations.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_stop() {
        let handle = ExecutionHandle::new(10);
        assert!(!handle.is_stop_requested());
        handle.stop();
        assert!(handle.is_stop_requested());
    }

    #[test]
    fn test_handle_pause_resume() {
        let handle = ExecutionHandle::new(10);
        assert!(!handle.is_paused());
        handle.pause();
        assert!(handle.is_paused());
        handle.resume();
        assert!(!handle.is_paused());
    }

    #[test]
    fn test_handle_cancel() {
        let handle = ExecutionHandle::new(10);
        assert!(!handle.is_cancelled());
        assert!(!handle.is_stop_requested());
        handle.cancel();
        assert!(handle.is_cancelled());
        assert!(handle.is_stop_requested()); // Cancel also sets stop
    }

    #[test]
    fn test_handle_iterations() {
        let handle = ExecutionHandle::new(10);
        assert_eq!(handle.current_iteration(), 0);
        assert_eq!(handle.max_iterations(), 10);

        handle.increment();
        assert_eq!(handle.current_iteration(), 1);

        handle.add_iterations(5);
        assert_eq!(handle.max_iterations(), 15);
    }

    #[test]
    fn stop_signal_returns_clone_of_underlying_flag() {
        let handle = ExecutionHandle::new(10);
        let signal = handle.stop_signal();

        // The clone observes the same flag — handle.stop() flips it.
        assert!(!signal.load(Ordering::SeqCst));
        handle.stop();
        assert!(signal.load(Ordering::SeqCst));

        // Conversely, mutating via the signal flips what the handle sees.
        let handle2 = ExecutionHandle::new(10);
        let signal2 = handle2.stop_signal();
        signal2.store(true, Ordering::SeqCst);
        assert!(handle2.is_stop_requested());
    }
}
