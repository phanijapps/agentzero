//! # Gateway Execution
//!
//! Agent execution engine for the AgentZero gateway.
//!
//! This crate provides the execution layer that:
//! - Creates and manages agent executors
//! - Converts execution events to gateway events
//! - Broadcasts events to connected clients
//! - Handles agent delegation with callbacks
//! - Spawns continuation turns after delegations complete
//!
//! ## Module Structure
//!
//! - `config` - Execution configuration and file system context
//! - `continuation` - Continuation spawning after delegation completion
//! - `handle` - Execution control handle (stop, pause, resume, cancel)
//! - `events` - Event conversion from runtime to gateway events
//! - `delegation` - Agent-to-agent delegation with callbacks
//! - `invoke` - Setup and executor building utilities
//! - `lifecycle` - Session and execution state management
//! - `runner` - Main execution runner

pub mod a2a;
pub mod agent_pool;
pub mod archiver;
pub mod artifacts;
pub mod composite_provider;
pub mod config;
pub mod continuation;
pub mod conversation_history;
pub mod curator;
pub mod delegation;
pub mod errors;
pub mod events;
pub mod handle;
pub mod indexer;
pub mod ingest;
pub mod invoke;
pub mod lifecycle;
pub mod middleware;
pub mod peer_messaging;
pub mod recall;
pub mod resource_provider;
pub mod runner;
pub mod session_ctx;
pub mod session_state;
pub mod session_title;
pub mod sleep;
#[cfg(test)]
pub(crate) mod test_stores;
pub mod tool_result_extractor;
pub mod tools;
pub mod ward_artifact_indexer;

// Re-export public types
pub use archiver::SessionArchiver;
pub use composite_provider::CompositeResourceProvider;
pub use config::{ExecutionConfig, GatewayFileSystem};
pub use continuation::{check_and_spawn_continuation, spawn_continuation_turn};
pub use delegation::{
    handle_delegation_failure, handle_delegation_success, handle_subagent_completion,
    spawn_delegated_agent, DelegationContext, DelegationRegistry, DelegationRequest,
};
pub use distillation::SessionDistiller;
pub use events::convert_stream_event;
pub use handle::ExecutionHandle;
pub use lifecycle::{
    complete_execution, crash_execution, emit_agent_started, emit_delegation_completed,
    emit_delegation_started, get_or_create_session, start_execution, stop_execution, SessionSetup,
};
pub use recall::{
    build_context_packet, build_context_packet_from_atoms, format_scored_items,
    format_scored_items_with_options, render_context_packet, ContextPacketBuildOptions,
    MemoryRecall,
};
pub use resource_provider::GatewayResourceProvider;
pub use runner::{ExecutionRunner, ExecutionRunnerConfig, OnSessionReady};
pub use session_state::{SessionState, SessionStateBuilder};
