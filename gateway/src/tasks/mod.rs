//! Durable + A2A task subsystem.
//!
//! Extracted from the gateway shell root (W1 of the gateway decompose —
//! see docs/gateway-decompose-deck.html). The two task surfaces are
//! siblings by necessity: a2a tasks land on the durable agent-task
//! target, and the durable layer is the only writer of the agent-task
//! work items. They stay gateway-internal modules because both depend
//! on gateway-local types (`services::RuntimeService`, `events`,
//! `hooks`); promoting them to services/ crates would drag those out
//! first — a later wave's decision, not this one's.
//!
//! - [`a2a`]: cross-agent A2A task lifecycle (create/claim/complete,
//!   peer identity, remote invocation via `gateway-a2a`).
//! - [`durable_agent`]: the durable agent-task queue surface (drafts,
//!   validation, retry policy, agent-task work items).

pub mod a2a;
pub mod durable_agent;

pub use a2a::*;
pub use durable_agent::*;
