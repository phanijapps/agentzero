//! Runtime SQLite persistence for zbot sessions and executions.
//!
//! This crate owns the `conversations.db` connection pool, schema, and
//! operational distillation-run persistence.

mod connection;
mod distillation_repository;
mod distillation_store;
mod schema;
pub mod system_profile;

pub use connection::DatabaseManager;
pub use distillation_repository::{
    DistillationRepository, DistillationRun, DistillationStats, UndistilledSession,
};
pub use distillation_store::GatewayDistillationStore;
