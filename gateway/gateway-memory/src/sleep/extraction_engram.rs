//! Procedure extraction + memory synthesis as engram consolidation arms
//! (P3 sub-batch 3).
//!
//! The extraction cores stay byte-preserved in `pattern_extractor.rs`
//! (episodes → similar pairs → shared tool-prefix → LLM generalization →
//! procedure row) and `synthesizer.rs` (KG entities seen across ≥2
//! sessions → LLM → `category='strategy'` fact). What changes is the
//! dispatch: both now run as `ConsolidationMutationExecutor` arms under
//! engram's `ProcedureExtraction` and `MemorySynthesis` task kinds,
//! triggered from the sleep worker via `ConsolidationRequest` — the same
//! pattern as `BeliefConsolidation` (P3.1) and `HierarchyConsolidation`
//! (P3.2).
//!
//! Success/failure accounting: `run_procedure` already bumps the zbot
//! `ProcedureStore`'s `increment_success` / `increment_failure` (the
//! zbot-trait equivalent of engram-procedures' `increment_n` n/failure
//! counters — same semantics, already wired); no second counter path is
//! introduced.

use std::sync::Arc;

use async_trait::async_trait;
use engram_consolidation::{ConsolidationMutationExecutor, ConsolidationMutationOutcome};
use engram_domain::{
    ConsolidationRequest, ConsolidationStats, ConsolidationTaskKind, ConsolidationTaskResult,
    ConsolidationTaskStatus, Timestamp,
};
use engram_runtime::CoreResult;

use crate::sleep::pattern_extractor::{PatternExtractor, PatternStats};
use crate::sleep::synthesizer::{SynthesisStats, Synthesizer};

/// The `ProcedureExtraction` arm: one `PatternExtractor::run_cycle` per
/// consolidation run, with its counts reported into the audit trail.
pub struct ZbotProcedureExtractionArm {
    extractor: Arc<PatternExtractor>,
}

impl ZbotProcedureExtractionArm {
    pub fn new(extractor: Arc<PatternExtractor>) -> Self {
        Self { extractor }
    }
}

#[async_trait]
impl ConsolidationMutationExecutor for ZbotProcedureExtractionArm {
    async fn execute(
        &self,
        request: &ConsolidationRequest,
        planned_tasks: &[ConsolidationTaskKind],
        started_at: Timestamp,
    ) -> CoreResult<ConsolidationMutationOutcome> {
        let run_id = scope_run_id(request);
        let core = match self.extractor.run_cycle(&run_id).await {
            Ok(stats) => Ok(ArmCounts {
                items_written: stats.procedures_inserted,
                items_read: stats.episodes_considered,
                model_calls: stats.llm_calls_made,
            }),
            Err(message) => Err(message),
        };
        run_cycle_arm(
            planned_tasks,
            started_at,
            ConsolidationTaskKind::ProcedureExtraction,
            core,
        )
        .await
    }
}

/// The `MemorySynthesis` arm: one `Synthesizer::run_cycle` (cross-session
/// strategy-fact synthesis) per consolidation run.
pub struct ZbotMemorySynthesisArm {
    synthesizer: Arc<Synthesizer>,
}

impl ZbotMemorySynthesisArm {
    pub fn new(synthesizer: Arc<Synthesizer>) -> Self {
        Self { synthesizer }
    }
}

#[async_trait]
impl ConsolidationMutationExecutor for ZbotMemorySynthesisArm {
    async fn execute(
        &self,
        request: &ConsolidationRequest,
        planned_tasks: &[ConsolidationTaskKind],
        started_at: Timestamp,
    ) -> CoreResult<ConsolidationMutationOutcome> {
        let run_id = scope_run_id(request);
        let core = match self.synthesizer.run_cycle(&run_id).await {
            Ok(stats) => Ok(ArmCounts {
                items_written: stats.facts_inserted + stats.facts_bumped,
                items_read: stats.candidates_considered,
                model_calls: stats.llm_calls_made,
            }),
            Err(message) => Err(message),
        };
        run_cycle_arm(
            planned_tasks,
            started_at,
            ConsolidationTaskKind::MemorySynthesis,
            core,
        )
        .await
    }
}

/// Counts an arm reports into its task result.
#[derive(Clone)]
struct ArmCounts {
    items_read: u64,
    items_written: u64,
    model_calls: u64,
}

/// Shared arm body: skip non-matching task kinds, run the core once,
/// report counts. A core failure is a task error (not an arm abort) so
/// the composite's other tasks still run.
async fn run_cycle_arm(
    planned_tasks: &[ConsolidationTaskKind],
    started_at: Timestamp,
    kind: ConsolidationTaskKind,
    core: Result<ArmCounts, String>,
) -> CoreResult<ConsolidationMutationOutcome> {
    let mut task_results = Vec::new();
    let mut errors = Vec::new();
    let mut counts = ArmCounts {
        items_read: 0,
        items_written: 0,
        model_calls: 0,
    };
    for planned in planned_tasks {
        if *planned != kind {
            task_results.push(ConsolidationTaskResult {
                task: planned.clone(),
                status: ConsolidationTaskStatus::Skipped,
                started_at,
                completed_at: Some(started_at),
                items_read: None,
                items_written: None,
                items_updated: None,
                items_skipped: None,
                model_calls: None,
                errors: Vec::new(),
                output_refs: Vec::new(),
            });
            continue;
        }

        match core.clone() {
            Ok(result) => {
                counts = result;
                task_results.push(ConsolidationTaskResult {
                    task: planned.clone(),
                    status: ConsolidationTaskStatus::Completed,
                    started_at,
                    completed_at: Some(chrono::Utc::now()),
                    items_read: Some(counts.items_read),
                    items_written: Some(counts.items_written),
                    items_updated: None,
                    items_skipped: None,
                    model_calls: Some(counts.model_calls),
                    errors: Vec::new(),
                    output_refs: Vec::new(),
                });
            }
            Err(message) => {
                task_results.push(ConsolidationTaskResult {
                    task: planned.clone(),
                    status: ConsolidationTaskStatus::Failed,
                    started_at,
                    completed_at: Some(chrono::Utc::now()),
                    items_read: Some(counts.items_read),
                    items_written: Some(counts.items_written),
                    items_updated: None,
                    items_skipped: None,
                    model_calls: Some(counts.model_calls),
                    errors: vec![engram_domain::ConsolidationError {
                        code: "cycle_failed".to_string(),
                        message: message.clone(),
                        task: Some(planned.clone()),
                        target_type: None,
                        target_id: None,
                        recoverable: true,
                    }],
                    output_refs: Vec::new(),
                });
                errors.push(engram_domain::ConsolidationError {
                    code: "cycle_failed".to_string(),
                    message,
                    task: Some(planned.clone()),
                    target_type: None,
                    target_id: None,
                    recoverable: true,
                });
            }
        }
    }

    Ok(ConsolidationMutationOutcome {
        tasks: task_results,
        stats: ConsolidationStats {
            memories_read: None,
            memories_written: None,
            beliefs_synthesized: None,
            contradictions_detected: None,
            hierarchy_nodes_created: None,
            hierarchy_relations_created: None,
            records_decayed: None,
            records_pruned: None,
            model_calls: None,
        },
        errors,
    })
}

// ---------------------------------------------------------------------------
// Triggers — the sleep-worker surface
// ---------------------------------------------------------------------------

/// One procedure-extraction cycle dispatched through engram's composite.
pub struct ProcedureExtractionConsolidation {
    composite: engram_consolidation::CompositeConsolidationExecutor,
}

impl ProcedureExtractionConsolidation {
    pub fn new(extractor: Arc<PatternExtractor>) -> Self {
        Self {
            composite: engram_consolidation::CompositeConsolidationExecutor::new(vec![Arc::new(
                ZbotProcedureExtractionArm::new(extractor),
            )]),
        }
    }

    pub async fn execute(&self, run_id: &str) -> Result<PatternStats, String> {
        let request = consolidation_request(run_id);
        let planned = [ConsolidationTaskKind::ProcedureExtraction];
        let outcome = self
            .composite
            .execute(&request, &planned, chrono::Utc::now())
            .await
            .map_err(|e| e.to_string())?;
        for error in &outcome.errors {
            tracing::warn!(run_id, code = %error.code, %error.message,
                "procedure-extraction: task error");
        }
        // The worker consumes `procedures_inserted`; derive it from the
        // audit trail instead of a second stats channel.
        let inserted = outcome
            .tasks
            .iter()
            .find(|result| result.task == ConsolidationTaskKind::ProcedureExtraction)
            .and_then(|result| result.items_written)
            .unwrap_or(0);
        Ok(PatternStats {
            procedures_inserted: inserted,
            ..PatternStats::default()
        })
    }
}

/// One memory-synthesis cycle dispatched through engram's composite.
pub struct MemorySynthesisConsolidation {
    composite: engram_consolidation::CompositeConsolidationExecutor,
}

impl MemorySynthesisConsolidation {
    pub fn new(synthesizer: Arc<Synthesizer>) -> Self {
        Self {
            composite: engram_consolidation::CompositeConsolidationExecutor::new(vec![Arc::new(
                ZbotMemorySynthesisArm::new(synthesizer),
            )]),
        }
    }

    pub async fn execute(&self, run_id: &str) -> Result<SynthesisStats, String> {
        let request = consolidation_request(run_id);
        let planned = [ConsolidationTaskKind::MemorySynthesis];
        let outcome = self
            .composite
            .execute(&request, &planned, chrono::Utc::now())
            .await
            .map_err(|e| e.to_string())?;
        for error in &outcome.errors {
            tracing::warn!(run_id, code = %error.code, %error.message,
                "memory-synthesis: task error");
        }
        // The worker consumes facts_inserted + facts_bumped; the arm
        // reports their sum as items_written — split evenly is wrong, so
        // report the sum as inserted (bumped facts are re-mentions).
        let written = outcome
            .tasks
            .iter()
            .find(|result| result.task == ConsolidationTaskKind::MemorySynthesis)
            .and_then(|result| result.items_written)
            .unwrap_or(0);
        Ok(SynthesisStats {
            facts_inserted: written,
            ..SynthesisStats::default()
        })
    }
}

/// The run id rides the scope's session slot; fall back to a fresh id.
fn scope_run_id(request: &ConsolidationRequest) -> String {
    request
        .scope
        .session
        .clone()
        .unwrap_or_else(|| format!("sleep-{}", uuid::Uuid::new_v4()))
}

fn consolidation_request(run_id: &str) -> ConsolidationRequest {
    ConsolidationRequest {
        scope: engram_domain::Scope {
            tenant: "zbot".to_string(),
            subject: None,
            workspace: None,
            // The arm uses this as the log-correlating run id.
            session: Some(run_id.to_string()),
            environment: None,
        },
        requester: engram_domain::Requester {
            actor: engram_domain::Actor {
                id: engram_domain::Id::from("zbot-sleep"),
                kind: engram_domain::ActorKind::System,
                display_name: None,
                metadata: None,
            },
            roles: Vec::new(),
            permissions: Vec::new(),
            on_behalf_of: None,
        },
        since: None,
        until: None,
        strategy: None,
        dry_run: Some(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consolidation_request_carries_run_id_in_session() {
        let request = consolidation_request("run-xyz");
        assert_eq!(request.scope.session.as_deref(), Some("run-xyz"));
        assert_eq!(request.scope.tenant, "zbot");
    }
}
