//! # Invoke Module
//!
//! Stream event processing, context, and execution accumulation for agent invocation.

mod batch_writer;
mod builder;
mod delegation_handler;
mod event_logging;
mod executor;
pub mod goal_adapter;
pub mod ingest_adapter;
pub mod kg_store_adapter;
pub mod micro_recall;
mod policy;
mod response_accumulator;
pub mod setup;
mod stream_context;
mod stream_event_processor;
mod token_tracking;
mod tool_call_accumulator;
mod tool_catalog;
pub mod unified_recall_adapter;
mod ward_layout_adapter;
mod ward_scaffolding;
pub mod ward_usage_adapter;
pub mod working_memory;
pub mod working_memory_middleware;

pub use batch_writer::{spawn_batch_writer, spawn_batch_writer_with_traces, BatchWriterHandle};
pub(crate) use executor::mcp_startup_failure_observer;
pub use executor::{
    build_context_capability_catalog, build_execution_engine, collect_agents_summary,
    collect_skills_summary, resolve_thinking_flag, ExecutorBuilder, RuntimeActorKind,
};
pub use micro_recall::{
    detect_triggers, execute_micro_recall, extract_new_entities, MicroRecallContext,
    MicroRecallTrigger,
};
pub(crate) use response_accumulator::assistant_turn_content;
pub use response_accumulator::ResponseAccumulator;
pub use setup::{
    append_system_context, detect_subagent_role, subagent_rules, AgentLoader, SubagentRole,
};
pub use stream_context::StreamContext;
pub use stream_event_processor::{broadcast_event, process_stream_event};
pub(crate) use stream_event_processor::{build_session_plan_surface, persist_gateway_surface};
pub use tool_call_accumulator::{ToolCallAccumulator, ToolCallRecord};
pub use ward_scaffolding::{collect_ward_setup_for_skill, collect_ward_setups_for_skills};
pub use working_memory::WorkingMemory;
