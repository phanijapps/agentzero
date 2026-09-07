//! # Delegation Module
//!
//! Handles agent-to-agent delegation with fire-and-forget pattern
//! and callback completion notifications.
//!
//! ## Module Structure
//!
//! - `context` - Delegation context and request types
//! - `registry` - Registry for tracking active delegations
//! - `callback` - Callback formatting and sending
//! - `spawn` - Delegated agent spawning logic

mod callback;
mod context;
mod registry;
pub mod spawn;
// Re-export public types
pub use callback::{
    extract_structured_result, format_agent_display_name, format_callback_message,
    format_error_callback_message, handle_delegation_failure, handle_delegation_success,
    handle_subagent_completion, send_callback_to_parent,
};
pub use context::{DelegationContext, DelegationMode, DelegationRequest};
pub use registry::DelegationRegistry;
pub use spawn::spawn_delegated_agent;
