//! `zbot-conversation` — the conversation store (messages + versioned
//! checkpoints) behind narrow traits. See
//! `docs/specs/conversation-store-revamp/spec.md`.

pub mod domain;
pub mod checkpoints;
pub mod messages;
pub mod schema;
mod pool;

pub use domain::{Checkpoint, Message};
pub use checkpoints::{CheckpointStore, SqliteCheckpointStore};
pub use messages::{MessageStore, SqliteMessageStore};
pub use pool::open_conversation_pool;
