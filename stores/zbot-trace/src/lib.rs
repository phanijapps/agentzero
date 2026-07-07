//! `zbot-trace` — the trace store: slim `execution_logs` (payload-free),
//! streamed per-session `.jsonl.zst` full-fidelity events, and DuckDB
//! analytics. See `docs/specs/conversation-store-revamp/spec.md`.

pub mod domain;
pub mod schema;
pub mod slim_logs;
mod pool;

pub use domain::{SlimLog, TraceEvent};
pub use pool::open_trace_pool;
pub use slim_logs::{SlimLogStore, SqliteSlimLogStore};
