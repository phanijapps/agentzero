//! Sleep-time worker — hourly background maintenance of the knowledge graph.
//!
//! Operations: compact duplicates, decay stale non-archival entities,
//! prune orphan candidates. Archival entities are exempt from every op.
//!
//! Tasks 5, 6, and 7 extend this module with `decay`, `pruner`, and `worker`.

pub mod compactor;
pub mod conflict_resolver;
pub mod corrections_abstractor;
pub mod decay;
pub mod handoff_writer;
pub mod orphan_archiver;
pub mod pattern_extractor;
pub mod pruner;
pub mod synthesizer;
pub mod verifier;
pub mod worker;

pub use compactor::{CompactionStats, Compactor, PairwiseVerifier};
pub use conflict_resolver::{
    ConflictJudgeLlm, ConflictResolver, ConflictResponse, ConflictStats, LlmConflictJudge,
};
pub use corrections_abstractor::{
    AbstractionLlm, AbstractionStats, CorrectionsAbstractor, LlmCorrectionsAbstractor,
};
pub use decay::{DecayConfig, DecayEngine, PruneCandidate};
pub use handoff_writer::{
    read_handoff_block, should_inject, HandoffEntry, HandoffInput, HandoffLlm, HandoffWriter,
    LlmHandoffWriter,
};
pub use orphan_archiver::{OrphanArchiver, OrphanArchiverStats};
pub use pattern_extractor::{
    LlmPatternExtractor, PatternExtractLlm, PatternExtractor, PatternResponse, PatternStats,
};
pub use pruner::{PruneStats, Pruner};
pub use synthesizer::{
    LlmSynthesizer, SynthesisInput, SynthesisLlm, SynthesisResponse, SynthesisStats, Synthesizer,
};
pub use verifier::LlmPairwiseVerifier;
pub use worker::{SleepOps, SleepTimeWorker};
