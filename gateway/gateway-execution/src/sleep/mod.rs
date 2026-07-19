//! Execution-specific sleep helpers.
//!
//! Sleep maintenance operations live in `gateway_memory::sleep`. This module
//! keeps handoff writing because it depends on execution-layer conversation and
//! prompt conventions.

pub mod handoff_writer;

pub use handoff_writer::{
    read_handoff_block, should_inject, HandoffEntry, HandoffInput, HandoffLlm, HandoffWriter,
    LlmHandoffWriter,
};
