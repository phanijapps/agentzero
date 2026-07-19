use engram_domain::{CapabilityReason, CapabilityState};
use engram_integration::CapabilityReport as EngramCapabilityReport;
use zbot_engram_adapter::{
    recall::{RecallBlocker, RecallParityArtifact, RecallSupportReport},
    AdapterConfig, AdapterFeature, EngramProvider,
};

#[test]
fn recall_gate_reports_retrieval_port_blocker() {
    let root = tempfile::tempdir().expect("root");
    let provider = EngramProvider::open(AdapterConfig::engram_for_data_root(root.path(), "engram"))
        .expect("provider");

    let report = RecallSupportReport::from_provider(&provider, None);

    assert!(!report.supported);
    assert_eq!(
        report.blocker,
        Some(RecallBlocker::RetrievalPortUnsupported)
    );
    assert!(!provider.capabilities().supports(AdapterFeature::Recall));
}

#[test]
fn recall_gate_requires_ranking_trace_parity_artifact() {
    let upstream = EngramCapabilityReport::builder()
        .retrieval(CapabilityState::Supported)
        .memory(CapabilityState::Supported)
        .knowledge(CapabilityState::Supported)
        .vectors(CapabilityState::Supported)
        .build();

    let missing = RecallSupportReport::from_upstream_capabilities(&upstream, None);
    assert!(!missing.supported);
    assert_eq!(
        missing.blocker,
        Some(RecallBlocker::MissingRankingTraceParityArtifact)
    );

    let incomplete = RecallSupportReport::from_upstream_capabilities(
        &upstream,
        Some(&RecallParityArtifact {
            ordered_candidates: true,
            scores: true,
            source_labels: false,
            trace_fields: true,
        }),
    );
    assert!(!incomplete.supported);
    assert_eq!(
        incomplete.blocker,
        Some(RecallBlocker::MissingRankingTraceParityArtifact)
    );

    let complete = RecallSupportReport::from_upstream_capabilities(
        &upstream,
        Some(&RecallParityArtifact {
            ordered_candidates: true,
            scores: true,
            source_labels: true,
            trace_fields: true,
        }),
    );
    assert!(complete.supported);
    assert_eq!(complete.blocker, None);
}

#[test]
fn recall_gate_treats_degraded_retrieval_as_unsupported() {
    let upstream = EngramCapabilityReport::builder()
        .retrieval(CapabilityState::Degraded {
            reason: CapabilityReason::DimensionMismatch,
        })
        .build();
    let complete = RecallParityArtifact {
        ordered_candidates: true,
        scores: true,
        source_labels: true,
        trace_fields: true,
    };

    let report = RecallSupportReport::from_upstream_capabilities(&upstream, Some(&complete));

    assert!(!report.supported);
    assert_eq!(
        report.blocker,
        Some(RecallBlocker::RetrievalPortUnsupported)
    );
}
