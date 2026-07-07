//! Stable store-trait parity case metadata shared by adapter-level gates.

/// Feature family covered by a conformance case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreTraitFeature {
    /// `MemoryFactStore` scenarios.
    MemoryFacts,
    /// `BeliefStore` scenarios.
    Beliefs,
}

/// A store-trait scenario that can be run by provider-specific fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreTraitCase {
    /// Stable fixture id used in adapter capability gates.
    pub id: &'static str,
    /// Feature family the case protects.
    pub feature: StoreTraitFeature,
    /// Exported conformance function that implements the scenario.
    pub function: &'static str,
}

/// Minimal memory fact scenario required before memory support can flip on.
pub const MEMORY_SAVE_AND_COUNT_ID: &str = "memory.save_and_count";

/// Minimal belief scenario required before belief support can flip on.
pub const BELIEF_UPSERT_GET_ID: &str = "belief.upsert_get";

const SEED_STORE_TRAIT_CASES: &[StoreTraitCase] = &[
    StoreTraitCase {
        id: MEMORY_SAVE_AND_COUNT_ID,
        feature: StoreTraitFeature::MemoryFacts,
        function: "memory_save_and_count",
    },
    StoreTraitCase {
        id: BELIEF_UPSERT_GET_ID,
        feature: StoreTraitFeature::Beliefs,
        function: "belief_upsert_get_round_trip",
    },
];

/// Accepted starting cases that every provider parity fixture registry must
/// track before a matching feature can report supported.
pub fn seed_store_trait_cases() -> &'static [StoreTraitCase] {
    SEED_STORE_TRAIT_CASES
}
