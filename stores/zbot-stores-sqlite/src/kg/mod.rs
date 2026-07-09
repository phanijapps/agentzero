//! SQLite-coupled knowledge-graph storage. Relocated from
//! `services/knowledge-graph/src/{storage,traversal,causal,service}.rs`
//! during Slice D6b of the persistence refactor (see
//! `docs/architecture/future-state/db-provider-portability.md`).
//!
//! `service.rs` came along with the storage move because it is a thin
//! wrapper over `GraphStorage` and could not stay in `services/knowledge-graph`
//! without re-introducing a circular dependency on this crate.

pub mod causal;
pub mod service;
pub mod storage;
pub mod traversal;
