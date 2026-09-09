//! The typed error for every store trait in this crate.
//!
//! Replaces the stringly `Result<_, String>` that every store signature
//! carried: callers can now match on the failure class (unavailable
//! backend vs. missing row vs. bad input vs. write conflict) instead of
//! substring-matching error prose. `From<String>` keeps migrations from
//! legacy impls mechanical — a plain string becomes `Backend`.

use std::fmt;

/// Failure classes shared by all store backends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// The store is not wired / its backend is down or disabled.
    Unavailable(String),
    /// The requested row/key/ward does not exist.
    NotFound(String),
    /// The caller-supplied data failed validation.
    Invalid(String),
    /// The write conflicts with existing state (duplicate key, stale
    /// version, guarded transition).
    Conflict(String),
    /// The backend rejected the operation (SQL failure, decode error,
    /// IO). Legacy string errors map here.
    Backend(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (class, detail) = match self {
            StoreError::Unavailable(d) => ("unavailable", d),
            StoreError::NotFound(d) => ("not found", d),
            StoreError::Invalid(d) => ("invalid", d),
            StoreError::Conflict(d) => ("conflict", d),
            StoreError::Backend(d) => ("backend", d),
        };
        write!(f, "store {class}: {detail}")
    }
}

impl std::error::Error for StoreError {}

impl From<String> for StoreError {
    fn from(detail: String) -> Self {
        StoreError::Backend(detail)
    }
}

impl From<&str> for StoreError {
    fn from(detail: &str) -> Self {
        StoreError::Backend(detail.to_string())
    }
}

impl StoreError {
    /// The error detail without the class prefix.
    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            StoreError::Unavailable(d)
            | StoreError::NotFound(d)
            | StoreError::Invalid(d)
            | StoreError::Conflict(d)
            | StoreError::Backend(d) => d,
        }
    }
}

/// Result alias used by every store trait signature.
pub type StoreResult<T> = Result<T, StoreError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_class_and_detail() {
        assert_eq!(
            StoreError::NotFound("ward ghost".into()).to_string(),
            "store not found: ward ghost"
        );
    }

    #[test]
    fn from_string_maps_to_backend() {
        let e: StoreError = "rusqlite: table missing".to_string().into();
        assert!(matches!(e, StoreError::Backend(_)));
        assert_eq!(e.detail(), "rusqlite: table missing");
    }

    #[test]
    fn variants_are_comparable() {
        assert_eq!(
            StoreError::Invalid("k".into()),
            StoreError::Invalid("k".into())
        );
        assert_ne!(
            StoreError::Invalid("k".into()),
            StoreError::Conflict("k".into())
        );
    }
}
