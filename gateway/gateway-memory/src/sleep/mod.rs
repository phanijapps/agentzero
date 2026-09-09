//! Sleep-time memory components — moved here from gateway/gateway-execution/src/sleep/
//! during the gateway-memory crate extraction (Phase B).

pub mod belief_engram;
pub mod belief_network_activity;
pub mod belief_propagator;
pub mod clustering;
pub mod compactor;
pub mod conflict_resolver;
pub mod decay;
pub mod extraction_engram;
pub mod hierarchy_engram;
pub mod llm_aggregate_entity;
pub mod orphan_archiver;
pub mod pattern_extractor;
pub mod pruner;
pub mod synthesizer;
pub mod verifier;
pub mod worker;

// Convenience re-exports so `crate::sleep::Compactor` etc. resolve inside
// gateway-memory (used by `worker.rs` and the `services` factory). External
// callers still hit the crate-root re-exports in `lib.rs`.
pub use belief_engram::{
    BeliefConsolidation, BeliefConsolidationParts, BeliefContradictionConfig,
    ContradictionDetectionStats, ContradictionJudgeLlm, ContradictionJudgeResponse, JudgeDecision,
    LlmContradictionJudge,
};
pub use belief_engram::{
    BeliefSynthesisLlm, BeliefSynthesisStats, LlmBeliefSynthesizer, SynthesisLlmResponse,
    ZbotBeliefSink, ZbotBeliefSynthesizer, ZbotContradictionArm, ZbotContradictionDetector,
};
pub use belief_network_activity::{
    RecentBeliefNetworkActivity, TimestampedContradictionStats, TimestampedPropagationStats,
    TimestampedSynthesisStats, RECENT_CAPACITY,
};
pub use belief_propagator::{BeliefPropagationStats, BeliefPropagator};
pub use compactor::{CompactionStats, Compactor, PairwiseVerifier};
pub use conflict_resolver::{
    ConflictJudgeLlm, ConflictResolver, ConflictResponse, ConflictStats, LlmConflictJudge,
};
pub use decay::{
    ContradictionPropagationConfig, ContradictionPropagationStats, DecayConfig, DecayEngine,
    PruneCandidate,
};
pub use extraction_engram::{
    MemorySynthesisConsolidation, ProcedureExtractionConsolidation, ZbotMemorySynthesisArm,
    ZbotProcedureExtractionArm,
};
pub use hierarchy_engram::{
    AggregateEntityLlm, AggregateMemberContext, AggregateResponse, HierarchyConfig,
    HierarchyConsolidation, HierarchyStats, StopReason, ZbotHierarchyBuildArm,
    ZbotHierarchyBuilder,
};
pub use llm_aggregate_entity::LlmAggregateEntity;
pub use orphan_archiver::{OrphanArchiver, OrphanArchiverStats};
pub use pattern_extractor::{
    LlmPatternExtractor, PatternExtractLlm, PatternExtractor, PatternResponse, PatternStats,
};
pub use pruner::{PruneStats, Pruner};
pub use synthesizer::{
    LlmSynthesizer, SynthesisInput, SynthesisLlm, SynthesisResponse, SynthesisStats, Synthesizer,
};
pub use verifier::LlmPairwiseVerifier;
pub use worker::{CycleStats, SleepOps, SleepTimeWorker};
