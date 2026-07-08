//! `zbot-conversation` — the conversation store (messages + versioned
//! checkpoints) behind narrow traits. See
//! `docs/specs/conversation-store-revamp/spec.md`.

pub mod checkpoints;
pub mod domain;
pub mod messages;
mod pool;
pub mod schema;
pub mod session_meta;

pub use checkpoints::{CheckpointStore, SqliteCheckpointStore};
pub use domain::{Checkpoint, Message};
pub use messages::{MessageStore, SqliteMessageStore};
pub use pool::open_conversation_pool;
pub use session_meta::{SessionMetaStore, SqliteSessionMetaStore};
