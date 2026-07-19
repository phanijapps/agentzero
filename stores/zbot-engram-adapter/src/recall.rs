//! Recall readiness gate for Engram-backed provider rollout.
//!
//! AgentZero's recall contract includes ranked candidates and trace fields that
//! are broader than a raw retrieval handle. This module keeps recall disabled
//! until Engram exposes retrieval and the adapter records parity evidence for
//! the ranking/trace surface.

use serde::{Deserialize, Serialize};

use crate::bootstrap::EngramProvider;

/// Evidence that AgentZero's recall ranking and trace contract has parity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecallParityArtifact {
    /// Candidate order matches the existing zbot ranking contract.
    pub ordered_candidates: bool,
    /// Candidate scores are present and comparable.
    pub scores: bool,
    /// Candidate source labels survive for UI/Observatory display.
    pub source_labels: bool,
    /// Trace fields required by recall observability are present.
    pub trace_fields: bool,
}

impl RecallParityArtifact {
    /// True only when every recall parity dimension is covered.
    pub fn is_complete(&self) -> bool {
        self.ordered_candidates && self.scores && self.source_labels && self.trace_fields
    }
}

/// Named blocker for recall support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallBlocker {
    /// Engram did not expose a supported retrieval port.
    RetrievalPortUnsupported,
    /// Retrieval exists, but ranking/trace parity evidence is missing.
    MissingRankingTraceParityArtifact,
}

/// Recall support decision for startup diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecallSupportReport {
    /// Whether recall may be enabled.
    pub supported: bool,
    /// Machine-readable reason when unsupported.
    pub blocker: Option<RecallBlocker>,
}

impl RecallSupportReport {
    /// Build a recall support report from the adapter provider.
    pub fn from_provider(
        provider: &EngramProvider,
        artifact: Option<&RecallParityArtifact>,
    ) -> Self {
        Self::from_upstream_capabilities(provider.upstream_capabilities(), artifact)
    }

    /// Build a recall support report from raw upstream capabilities.
    pub fn from_upstream_capabilities(
        upstream: &engram_integration::CapabilityReport,
        artifact: Option<&RecallParityArtifact>,
    ) -> Self {
        if !upstream.retrieval_supported() {
            return Self {
                supported: false,
                blocker: Some(RecallBlocker::RetrievalPortUnsupported),
            };
        }

        if !artifact.is_some_and(RecallParityArtifact::is_complete) {
            return Self {
                supported: false,
                blocker: Some(RecallBlocker::MissingRankingTraceParityArtifact),
            };
        }

        Self {
            supported: true,
            blocker: None,
        }
    }
}
