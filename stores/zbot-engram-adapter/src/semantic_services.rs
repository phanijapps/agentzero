//! Adapter-private access to additive Engram semantic services.
//!
//! These services deliberately stay below the gateway and model-tool layers.
//! Each call requires both an upstream capability state and a concrete handle,
//! then maps provider failures to stable, redacted adapter errors.

#![allow(dead_code)] // Prepared adapter-private surface; no runtime consumer is approved here.

use engram_domain::{ContextPayload, EvidenceTargetType, Provenance, RetrievalRequest, Scope};
use engram_integration::{
    BatchIngestRequest, BatchStatus, BatchStep, DiagnosticsSnapshot, ImportData, StepStatus,
    TransactionGuarantee, ValidationReport,
};

use crate::{
    bootstrap::EngramProvider,
    error::{AdapterError, AdapterResult},
};

/// Adapter-local readiness for additive Engram ports.
///
/// `UnifiedRecall` is intentionally distinct from the existing z-Bot
/// `AdapterFeature::Recall`, which remains governed by its own parity and
/// policy contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EngramSemanticService {
    UnifiedRecall,
    BatchIngest,
    Provenance,
    Migration,
    Observability,
}

impl EngramSemanticService {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::UnifiedRecall => "engram_unified_recall",
            Self::BatchIngest => "engram_batch_ingest",
            Self::Provenance => "engram_provenance",
            Self::Migration => "engram_migration",
            Self::Observability => "engram_observability",
        }
    }
}

/// A batch outcome that never exposes a provider error string to callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SafeBatchOutcome {
    pub guarantee: TransactionGuarantee,
    pub status: BatchStatus,
    pub steps: Vec<SafeBatchStepOutcome>,
}

/// One safe per-step status from a best-effort batch operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SafeBatchStepOutcome {
    pub step: BatchStep,
    pub status: StepStatus,
    /// Stable, path-free reason code when Engram reported a failed step.
    pub error_code: Option<&'static str>,
}

impl From<engram_integration::BatchOutcome> for SafeBatchOutcome {
    fn from(outcome: engram_integration::BatchOutcome) -> Self {
        Self {
            guarantee: outcome.guarantee,
            status: outcome.status,
            steps: outcome
                .steps
                .into_iter()
                .map(|step| SafeBatchStepOutcome {
                    step: step.step,
                    status: step.status,
                    error_code: step.error.map(|_| "engram_operation_failed"),
                })
                .collect(),
        }
    }
}

/// Capability-gated operations over the provider opened by the adapter.
pub(crate) struct EngramSemanticServices<'a> {
    provider: &'a EngramProvider,
}

impl<'a> EngramSemanticServices<'a> {
    pub(crate) fn new(provider: &'a EngramProvider) -> Self {
        Self { provider }
    }

    pub(crate) async fn recall(&self, request: RetrievalRequest) -> AdapterResult<ContextPayload> {
        self.provider
            .require_semantic_service(EngramSemanticService::UnifiedRecall)?;
        self.provider
            .provider
            .recall()
            .expect("readiness checked handle presence")
            .recall(request)
            .await
            .map_err(|_| operation_error("engram_unified_recall"))
    }

    pub(crate) async fn diagnostics(&self) -> AdapterResult<DiagnosticsSnapshot> {
        self.provider
            .require_semantic_service(EngramSemanticService::Observability)?;
        self.provider
            .provider
            .observability()
            .expect("readiness checked handle presence")
            .diagnostics()
            .await
            .map_err(|_| operation_error("engram_observability"))
    }

    pub(crate) fn schema_version(&self) -> AdapterResult<String> {
        self.provider
            .require_semantic_service(EngramSemanticService::Migration)?;
        self.provider
            .provider
            .migration()
            .expect("readiness checked handle presence")
            .schema_version()
            .map_err(|_| operation_error("engram_migration"))
    }

    pub(crate) fn adapter_version(&self) -> AdapterResult<String> {
        self.provider
            .require_semantic_service(EngramSemanticService::Migration)?;
        Ok(self
            .provider
            .provider
            .migration()
            .expect("readiness checked handle presence")
            .adapter_version())
    }

    pub(crate) fn dry_run_import(&self, input: &ImportData) -> AdapterResult<ValidationReport> {
        self.provider
            .require_semantic_service(EngramSemanticService::Migration)?;
        self.provider
            .provider
            .migration()
            .expect("readiness checked handle presence")
            .dry_run_import(input)
            .map_err(|_| operation_error("engram_migration"))
    }

    pub(crate) async fn ingest(
        &self,
        request: BatchIngestRequest,
    ) -> AdapterResult<SafeBatchOutcome> {
        self.provider
            .require_semantic_service(EngramSemanticService::BatchIngest)?;
        self.provider
            .provider
            .batch()
            .expect("readiness checked handle presence")
            .ingest(request)
            .await
            .map(SafeBatchOutcome::from)
            .map_err(|_| operation_error("engram_batch_ingest"))
    }

    pub(crate) async fn provenance_for(
        &self,
        target: EvidenceTargetType,
        id: &str,
        scope: &Scope,
    ) -> AdapterResult<Option<Provenance>> {
        self.provider
            .require_semantic_service(EngramSemanticService::Provenance)?;
        self.provider
            .provider
            .provenance()
            .expect("readiness checked handle presence")
            .provenance_for(target, id, scope)
            .await
            .map_err(|_| operation_error("engram_provenance"))
    }
}

fn operation_error(component: &'static str) -> AdapterError {
    AdapterError::Storage {
        component,
        reason: "Engram semantic operation failed".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engram_domain::{Actor, ActorKind, Requester};

    fn scope() -> Scope {
        Scope {
            tenant: "agentzero".to_string(),
            subject: None,
            workspace: None,
            session: None,
            environment: None,
        }
    }

    fn empty_import_data() -> ImportData {
        ImportData {
            memories: Vec::new(),
            knowledge_sources: Vec::new(),
            knowledge_documents: Vec::new(),
            knowledge_chunks: Vec::new(),
            knowledge_entities: Vec::new(),
            knowledge_relationships: Vec::new(),
            concept_schemes: Vec::new(),
            concepts: Vec::new(),
            beliefs: Vec::new(),
            hierarchy_nodes: Vec::new(),
            vectors: Vec::new(),
        }
    }

    #[test]
    fn batch_outcome_redacts_upstream_error_details() {
        let safe = SafeBatchOutcome::from(engram_integration::BatchOutcome::from_steps(vec![
            engram_integration::StepOutcome::failed(
                BatchStep::Facts,
                engram_memory::CoreError::Adapter {
                    adapter: "/private/path/engram.db".to_string(),
                    message: "SQL failed".to_string(),
                },
            ),
        ]));

        assert_eq!(safe.status, BatchStatus::Partial);
        assert_eq!(safe.steps[0].error_code, Some("engram_operation_failed"));
        assert!(!format!("{safe:?}").contains("/private/path"));
        assert!(!format!("{safe:?}").contains("SQL failed"));
    }

    #[test]
    fn operation_errors_are_path_free() {
        let error = operation_error("engram_provenance");
        assert_eq!(error.kind(), crate::AdapterErrorKind::Storage);
        assert!(!error.to_string().contains("/"));
        assert!(!error.to_string().contains("SQL"));
    }

    #[tokio::test]
    async fn supported_services_are_adapter_private_and_operational() {
        let root = tempfile::tempdir().expect("root");
        let provider = EngramProvider::open(crate::AdapterConfig::engram_for_data_root(
            root.path(),
            "engram",
        ))
        .expect("provider");
        let services = provider.semantic_services();

        assert!(provider.semantic_service_ready(EngramSemanticService::UnifiedRecall));
        assert!(provider.semantic_service_ready(EngramSemanticService::BatchIngest));
        assert!(provider.semantic_service_ready(EngramSemanticService::Provenance));
        assert!(provider.semantic_service_ready(EngramSemanticService::Migration));
        assert!(provider.semantic_service_ready(EngramSemanticService::Observability));

        let diagnostics = services.diagnostics().await.expect("diagnostics");
        assert!(diagnostics.capabilities.observability_supported());
        assert!(!services
            .schema_version()
            .expect("schema version")
            .is_empty());
        assert!(!services
            .adapter_version()
            .expect("adapter version")
            .is_empty());
        assert!(
            services
                .dry_run_import(&empty_import_data())
                .expect("dry run")
                .target_path_valid
        );

        let request = RetrievalRequest {
            query: "semantic service test".to_string(),
            scope: scope(),
            requester: Requester {
                actor: Actor {
                    id: "adapter-test".into(),
                    kind: ActorKind::Service,
                    display_name: None,
                    metadata: None,
                },
                roles: Vec::new(),
                permissions: Vec::new(),
                on_behalf_of: None,
            },
            modes: Vec::new(),
            filters: None,
            cues: Vec::new(),
            limit: Some(5),
            budget: None,
            include_explanations: Some(false),
        };
        services.recall(request).await.expect("bounded recall");
        let batch = services
            .ingest(BatchIngestRequest {
                idempotency_key: "semantic-services-empty-batch".to_string(),
                scope: scope(),
                source: None,
                documents: Vec::new(),
                chunks: Vec::new(),
                facts: Vec::new(),
                entities: Vec::new(),
                relationships: Vec::new(),
                evidence: Vec::new(),
                embeddings: Vec::new(),
            })
            .await
            .expect("batch");
        assert_eq!(batch.guarantee, TransactionGuarantee::BestEffort);
        assert_eq!(batch.steps.len(), 6);
        assert!(batch.steps.iter().all(|step| step.error_code.is_none()));
        assert!(services
            .provenance_for(EvidenceTargetType::Entity, "missing", &scope())
            .await
            .expect("scoped provenance")
            .is_none());
    }
}
