//! `zbot-trace` — the trace store: slim `execution_logs` (payload-free),
//! streamed per-session `.jsonl.gz` full-fidelity events, and DuckDB
//! analytics. See `docs/specs/conversation-store-revamp/spec.md`.

pub mod analytics;
pub mod domain;
mod pool;
pub mod schema;
pub mod slim_logs;
pub mod writer;

pub use analytics::TraceAnalytics;
pub use domain::{SlimLog, TraceEvent};
pub use pool::open_trace_pool;
pub use slim_logs::{SlimLogStore, SqliteSlimLogStore};
pub use writer::TraceWriter;
