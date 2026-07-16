use async_trait::async_trait;

/// Bounded request for SKOS-style recall query expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecallTaxonomyExpansionRequest {
    pub query: String,
    pub ward_id: Option<String>,
    /// Trusted session identity carried from the gateway authorization seam.
    pub session_id: Option<String>,
    pub max_depth: u8,
    pub max_fan_out: u16,
    pub max_candidates: u16,
}

/// One non-secret expansion cue used by recall and trace telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecallTaxonomyExpansionCandidate {
    pub scheme_id: String,
    pub concept_id: String,
    pub label: String,
    pub matched_label: String,
    pub relation: Option<String>,
    pub depth: u8,
}

/// Result of bounded taxonomy expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecallTaxonomyExpansion {
    pub expanded_query: String,
    pub candidates: Vec<RecallTaxonomyExpansionCandidate>,
}

/// Optional taxonomy-backed recall expansion surface.
#[async_trait]
pub trait RecallTaxonomyExpander: Send + Sync {
    /// Whether this trusted runtime scope has an active taxonomy selection.
    ///
    /// The default keeps existing simple expanders source-compatible. Real
    /// providers override it so a ward/session that has no selected scheme is
    /// reported as `not_configured`, not as an empty taxonomy result.
    fn is_configured_for(&self, _ward_id: Option<&str>, _session_id: Option<&str>) -> bool {
        true
    }

    async fn expand_recall_query(
        &self,
        request: RecallTaxonomyExpansionRequest,
    ) -> Result<RecallTaxonomyExpansion, String>;
}
