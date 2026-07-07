//! # Invoke Module
//!
//! Stream event processing, context, and execution accumulation for agent invocation.

mod batch_writer;
mod delegation_handler;
mod event_logging;
mod executor;
pub mod goal_adapter;
pub mod ingest_adapter;
pub mod kg_store_adapter;
pub mod micro_recall;
mod response_accumulator;
pub mod setup;
mod stream_context;
mod stream_event_processor;
mod token_tracking;
mod tool_call_accumulator;
mod ward_scaffolding;
pub mod ward_usage_adapter;
pub mod working_memory;
pub mod working_memory_middleware;

pub use batch_writer::{spawn_batch_writer, spawn_batch_writer_with_repo, BatchWriterHandle};
pub use executor::{
    build_context_capability_catalog, collect_agents_summary, collect_skills_summary,
    resolve_thinking_flag, select_engine, ExecutorBuilder, RuntimeActorKind,
};
pub use micro_recall::{
    detect_triggers, execute_micro_recall, extract_new_entities, MicroRecallContext,
    MicroRecallTrigger,
};
pub use response_accumulator::ResponseAccumulator;
pub use setup::{
    append_system_context, detect_subagent_role, subagent_rules, AgentLoader, SubagentRole,
};
pub use stream_context::StreamContext;
pub use stream_event_processor::{broadcast_event, process_stream_event};
pub use tool_call_accumulator::{ToolCallAccumulator, ToolCallRecord};
pub use ward_scaffolding::{collect_ward_setup_for_skill, collect_ward_setups_for_skills};
pub use working_memory::WorkingMemory;
