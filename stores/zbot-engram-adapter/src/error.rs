//! Typed adapter errors with trait-boundary string conversion.

use thiserror::Error;
use zbot_stores_traits::StoreError;

/// Result type used inside the adapter before translating to AgentZero traits.
pub type AdapterResult<T> = Result<T, AdapterError>;

/// Stable error class used by tests and capability gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterErrorKind {
    /// Missing required configuration.
    MissingConfig,
    /// Invalid host-to-Engram scope translation.
    InvalidScope,
    /// Configured path escapes the sanctioned data root.
    PathNotConfined,
    /// Engram store construction failed.
    Bootstrap,
    /// Mapping between AgentZero and Engram shapes failed.
    Mapping,
    /// Adapter-owned storage failed after bootstrap.
    Storage,
    /// Feature is unavailable in the selected provider mode.
    UnsupportedFeature,
}

/// Errors raised by adapter configuration, mapping, and compatibility checks.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AdapterError {
    /// A required configuration field was blank.
    #[error("missing required adapter config field: {field}")]
    MissingConfig {
        /// The missing field name.
        field: &'static str,
    },

    /// A scope component was empty or unsafe to translate.
    #[error("invalid scope component `{component}`: {reason}")]
    InvalidScope {
        /// Scope component name.
        component: &'static str,
        /// Human-readable reason.
        reason: String,
    },

    /// A configured path did not resolve under the sanctioned data root.
    #[error("configured path `{field}` is not confined to the zbot data root: {reason}")]
    PathNotConfined {
        /// Config field name.
        field: &'static str,
        /// Human-readable reason without raw private path details.
        reason: String,
    },

    /// Engram store construction failed.
    #[error("failed to bootstrap Engram `{component}` store: {reason}")]
    Bootstrap {
        /// Store component name.
        component: &'static str,
        /// Redacted reason.
        reason: String,
    },

    /// AgentZero data could not be mapped into or out of Engram contracts.
    #[error("failed to map `{field}` between AgentZero and Engram: {reason}")]
    Mapping {
        /// Field or payload being mapped.
        field: &'static str,
        /// Redacted reason.
        reason: String,
    },

    /// Adapter-owned storage operation failed after provider startup.
    #[error("adapter storage `{component}` failed: {reason}")]
    Storage {
        /// Adapter component name.
        component: &'static str,
        /// Redacted reason.
        reason: String,
    },

    /// A feature was requested but the adapter has not implemented it.
    #[error("adapter feature `{feature}` is unsupported: {reason}")]
    UnsupportedFeature {
        /// Feature name.
        feature: &'static str,
        /// Why it is unsupported.
        reason: String,
    },
}

impl AdapterError {
    /// Return the stable error class.
    pub fn kind(&self) -> AdapterErrorKind {
        match self {
            Self::MissingConfig { .. } => AdapterErrorKind::MissingConfig,
            Self::InvalidScope { .. } => AdapterErrorKind::InvalidScope,
            Self::PathNotConfined { .. } => AdapterErrorKind::PathNotConfined,
            Self::Bootstrap { .. } => AdapterErrorKind::Bootstrap,
            Self::Mapping { .. } => AdapterErrorKind::Mapping,
            Self::Storage { .. } => AdapterErrorKind::Storage,
            Self::UnsupportedFeature { .. } => AdapterErrorKind::UnsupportedFeature,
        }
    }

    /// Convert an internal error to the [`StoreError`] shape used by
    /// AgentZero store traits, preserving the failure class: unsupported
    /// features and bootstrap gaps surface as `Unavailable`, scope and
    /// mapping failures as `Invalid`, storage failures as `Backend`.
    pub fn into_trait_error(self) -> StoreError {
        match self.kind() {
            AdapterErrorKind::UnsupportedFeature | AdapterErrorKind::Bootstrap => {
                StoreError::Unavailable(self.to_string())
            }
            AdapterErrorKind::InvalidScope
            | AdapterErrorKind::PathNotConfined
            | AdapterErrorKind::Mapping
            | AdapterErrorKind::MissingConfig => StoreError::Invalid(self.to_string()),
            AdapterErrorKind::Storage => StoreError::Backend(self.to_string()),
        }
    }
}
