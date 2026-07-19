use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("backend unavailable (retry hint: {retry_after:?})")]
    Unavailable { retry_after: Option<Duration> },

    #[error("schema error: {0}")]
    Schema(String),

    #[error("backend error: {0}")]
    Backend(String),

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("config error: {0}")]
    Config(String),
}

pub type StoreResult<T> = Result<T, StoreError>;
