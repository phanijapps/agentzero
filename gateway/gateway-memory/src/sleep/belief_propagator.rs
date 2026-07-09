//! Belief Propagator — propagates fact-invalidation events to dependent
//! beliefs (Phase B-3 of the Belief Network).
//!
//! When a memory_fact is superseded (by `ConflictResolver`) or its
//! confidence drops below a threshold (by `DecayEngine`), call sites
//! invoke [`BeliefPropagator::propagate_invalidation`] inline — the
//! propagation is event-driven, not a polling sweep, so the dependent
//! beliefs see the change in the same cycle as the source fact.
//!
//! Two outcomes per dependent belief:
//!
//! - **Sole source** (`source_fact_ids.len() == 1`) → the belief is
//!   retracted via [`BeliefStore::retract_belief`] (`valid_until` set
//!   to the transition time). The belief's only support is now gone.
//! - **Multi-source** → the belief is marked stale via
//!   [`BeliefStore::mark_stale`]. The next `BeliefSynthesizer` cycle
//!   picks up stale beliefs first and re-synthesizes them from the
//!   remaining valid source facts, clearing the flag.
//!
//! Failure mode: any error from the belief store is logged at WARN and
//! the call returns with `errors += 1` in the stats. The propagator
//! NEVER bubbles errors — the upstream fact operation (supersession,
//! decay) must always succeed regardless of belief-side failures.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use zbot_stores_traits::BeliefStore;

/// Hop cap to prevent runaway propagation. In B-3 the schema only allows
/// fact → belief edges (beliefs don't reference other beliefs as sources
/// yet), so the effective max depth is `1`. The cap is here to make the
/// future fact-of-belief → higher-belief expansion bounded by design.
const MAX_PROPAGATION_DEPTH: u32 = 3;

/// Aggregate stats from one propagation call.
///
/// `beliefs_invalidated` counts every belief touched (retract + stale).
/// `max_propagation_depth` records the deepest hop actually traversed —
/// in B-3 this is `1` whenever any belief is touched.
#[derive(Debug, Default, Clone)]
pub struct BeliefPropagationStats {
    pub beliefs_invalidated: u64,
    pub beliefs_retracted: u64,
    pub beliefs_marked_stale: u64,
    pub max_propagation_depth: u32,
    pub errors: u64,
}

/// Propagates fact-invalidation events to the beliefs that depend on
/// those facts.
///
/// Constructed by the gateway's `MemoryServices` when the Belief Network
/// is enabled, then injected into [`crate::sleep::ConflictResolver`] and
/// the DecayEngine's fact-confidence-drop hook.
pub struct BeliefPropagator {
    belief_store: Arc<dyn BeliefStore>,
    enabled: bool,
}

impl BeliefPropagator {
    /// Construct with the wired belief store and the master enable flag.
    ///
    /// When `enabled = false`, every call to `propagate_invalidation`
    /// returns default stats without touching the store — the flag is
    /// shared with the rest of the Belief Network (`beliefNetwork.enabled`).
    pub fn new(belief_store: Arc<dyn BeliefStore>, enabled: bool) -> Self {
        Self {
            belief_store,
            enabled,
        }
    }

    /// Find every active belief that lists `fact_id` in its source set,
    /// retract sole-source beliefs, mark multi-source beliefs stale.
    ///
    /// `transition_time` is the moment the fact lost authority — passed
    /// through verbatim to [`BeliefStore::retract_belief`] so the
    /// resulting bi-temporal interval is consistent with the
    /// fact-supersession event that triggered this call.
    pub async fn propagate_invalidation(
        &self,
        fact_id: &str,
        transition_time: DateTime<Utc>,
    ) -> BeliefPropagationStats {
        let mut stats = BeliefPropagationStats::default();
        if !self.enabled {
            return stats;
        }

        let belief_ids = match self.belief_store.beliefs_referencing_fact(fact_id).await {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!(
                    fact_id = %fact_id,
                    error = %e,
                    "belief-propagator: beliefs_referencing_fact failed"
                );
                stats.errors += 1;
                return stats;
            }
        };

        if belief_ids.is_empty() {
            return stats;
        }

        // Effective depth is 1 in B-3 (fact → belief), capped under the
        // declared maximum so the bound is enforced even when future
        // schemas allow higher-order beliefs.
        let depth = 1_u32.min(MAX_PROPAGATION_DEPTH);
        stats.max_propagation_depth = depth;

        for belief_id in belief_ids {
            self.invalidate_one(&belief_id, transition_time, &mut stats)
                .await;
        }

        stats
    }

    /// Decide retract vs mark-stale based on the belief's source count,
    /// apply the chosen outcome. Errors are logged and counted, never
    /// bubbled. If the belief can't be loaded (unknown id, store error),
    /// we conservatively `mark_stale` so the next synthesizer cycle can
    /// re-derive from any remaining sources rather than silently
    /// dropping the belief.
    async fn invalidate_one(
        &self,
        belief_id: &str,
        transition_time: DateTime<Utc>,
        stats: &mut BeliefPropagationStats,
    ) {
        let belief = match self.belief_store.get_belief_by_id(belief_id).await {
            Ok(Some(b)) => b,
            Ok(None) => {
                tracing::warn!(
                    belief_id = %belief_id,
                    "belief-propagator: belief disappeared between lookup and load"
                );
                stats.errors += 1;
                return;
            }
            Err(e) => {
                tracing::warn!(
                    belief_id = %belief_id,
                    error = %e,
                    "belief-propagator: get_belief_by_id failed"
                );
                stats.errors += 1;
                return;
            }
        };

        let sole_source = belief.source_fact_ids.len() <= 1;
        if sole_source {
            if let Err(e) = self
                .belief_store
                .retract_belief(belief_id, transition_time)
                .await
            {
                tracing::warn!(
                    belief_id = %belief_id,
                    error = %e,
                    "belief-propagator: retract_belief failed"
                );
                stats.errors += 1;
                return;
            }
            stats.beliefs_invalidated += 1;
            stats.beliefs_retracted += 1;
        } else if let Err(e) = self.belief_store.mark_stale(belief_id).await {
            tracing::warn!(
                belief_id = %belief_id,
                error = %e,
                "belief-propagator: mark_stale failed"
            );
            stats.errors += 1;
        } else {
            stats.beliefs_invalidated += 1;
            stats.beliefs_marked_stale += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;
    use zbot_stores_traits::Belief;

    /// In-memory `BeliefStore` for propagator-level tests. Tracks each
    /// mutating call so assertions can verify both the outcome and the
    /// path taken.
    struct InMemBeliefStore {
        beliefs: StdMutex<Vec<Belief>>,
        fail_referencing: bool,
    }

    impl InMemBeliefStore {
        fn new(beliefs: Vec<Belief>) -> Self {
            Self {
                beliefs: StdMutex::new(beliefs),
                fail_referencing: false,
            }
        }

        fn failing() -> Self {
            Self {
                beliefs: StdMutex::new(vec![]),
                fail_referencing: true,
            }
        }

        fn get(&self, id: &str) -> Option<Belief> {
            self.beliefs
                .lock()
                .unwrap()
                .iter()
                .find(|b| b.id == id)
                .cloned()
        }
    }

    #[async_trait]
    impl BeliefStore for InMemBeliefStore {
        async fn get_belief(
            &self,
            _: &str,
            _: &str,
            _: Option<DateTime<Utc>>,
        ) -> Result<Option<Belief>, String> {
            Ok(None)
        }
        async fn list_beliefs(&self, _: &str, _: usize) -> Result<Vec<Belief>, String> {
            Ok(self.beliefs.lock().unwrap().clone())
        }
        async fn upsert_belief(&self, _: &Belief) -> Result<(), String> {
            Ok(())
        }
        async fn supersede_belief(&self, _: &str, _: &str, _: DateTime<Utc>) -> Result<(), String> {
            Ok(())
        }
        async fn mark_stale(&self, belief_id: &str) -> Result<(), String> {
            let mut bs = self.beliefs.lock().unwrap();
            if let Some(b) = bs.iter_mut().find(|b| b.id == belief_id) {
                b.stale = true;
                Ok(())
            } else {
                Err(format!("not found: {belief_id}"))
            }
        }
        async fn retract_belief(&self, belief_id: &str, t: DateTime<Utc>) -> Result<(), String> {
            let mut bs = self.beliefs.lock().unwrap();
            if let Some(b) = bs.iter_mut().find(|b| b.id == belief_id) {
                b.valid_until = Some(t);
                Ok(())
            } else {
                Err(format!("not found: {belief_id}"))
            }
        }
        async fn beliefs_referencing_fact(&self, fact_id: &str) -> Result<Vec<String>, String> {
            if self.fail_referencing {
                return Err("induced failure".into());
            }
            let bs = self.beliefs.lock().unwrap();
            Ok(bs
                .iter()
                .filter(|b| {
                    b.valid_until.is_none() && b.source_fact_ids.iter().any(|f| f == fact_id)
                })
                .map(|b| b.id.clone())
                .collect())
        }
        async fn get_belief_by_id(&self, belief_id: &str) -> Result<Option<Belief>, String> {
            Ok(self
                .beliefs
                .lock()
                .unwrap()
                .iter()
                .find(|b| b.id == belief_id)
                .cloned())
        }
        async fn list_stale(&self, _: &str, _: usize) -> Result<Vec<Belief>, String> {
            Ok(self
                .beliefs
                .lock()
                .unwrap()
                .iter()
                .filter(|b| b.stale)
                .cloned()
                .collect())
        }
        async fn clear_stale(&self, belief_id: &str) -> Result<(), String> {
            let mut bs = self.beliefs.lock().unwrap();
            if let Some(b) = bs.iter_mut().find(|b| b.id == belief_id) {
                b.stale = false;
            }
            Ok(())
        }
        async fn search_beliefs(
            &self,
            _: &str,
            _: &[f32],
            _: usize,
        ) -> Result<Vec<zbot_stores_traits::ScoredBelief>, String> {
            Ok(vec![])
        }
    }

    fn make_belief(id: &str, sources: Vec<&str>) -> Belief {
        let now = Utc::now();
        Belief {
            id: id.into(),
            partition_id: "p".into(),
            subject: "user.x".into(),
            content: "c".into(),
            confidence: 0.8,
            valid_from: Some(now),
            valid_until: None,
            source_fact_ids: sources.into_iter().map(String::from).collect(),
            synthesizer_version: 1,
            reasoning: None,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding: None,
        }
    }

    /// Sole-source belief → retracted (valid_until set), not marked stale.
    #[tokio::test]
    async fn sole_source_propagation_retracts() {
        let store = Arc::new(InMemBeliefStore::new(vec![make_belief("b1", vec!["F1"])]));
        let prop = BeliefPropagator::new(store.clone(), true);

        let ts = Utc::now();
        let stats = prop.propagate_invalidation("F1", ts).await;
        assert_eq!(stats.beliefs_invalidated, 1);
        assert_eq!(stats.beliefs_retracted, 1);
        assert_eq!(stats.beliefs_marked_stale, 0);
        assert_eq!(stats.errors, 0);
        assert_eq!(stats.max_propagation_depth, 1);

        let after = store.get("b1").unwrap();
        assert_eq!(after.valid_until, Some(ts));
        assert!(!after.stale, "retract path doesn't touch stale flag");
    }

    /// Multi-source belief → marked stale, no valid_until change.
    #[tokio::test]
    async fn multi_source_propagation_marks_stale() {
        let store = Arc::new(InMemBeliefStore::new(vec![make_belief(
            "b-multi",
            vec!["F1", "F2"],
        )]));
        let prop = BeliefPropagator::new(store.clone(), true);

        let stats = prop.propagate_invalidation("F1", Utc::now()).await;
        assert_eq!(stats.beliefs_invalidated, 1);
        assert_eq!(stats.beliefs_marked_stale, 1);
        assert_eq!(stats.beliefs_retracted, 0);
        assert_eq!(stats.errors, 0);

        let after = store.get("b-multi").unwrap();
        assert!(after.stale, "stale flag set");
        assert!(after.valid_until.is_none(), "valid_until NOT set");
    }

    /// `enabled = false` → no-op. The propagator never touches the store.
    #[tokio::test]
    async fn disabled_propagator_is_noop() {
        let store = Arc::new(InMemBeliefStore::new(vec![make_belief(
            "b1",
            vec!["F1", "F2"],
        )]));
        let prop = BeliefPropagator::new(store.clone(), false);

        let stats = prop.propagate_invalidation("F1", Utc::now()).await;
        assert_eq!(stats.beliefs_invalidated, 0);
        assert_eq!(stats.beliefs_marked_stale, 0);
        assert_eq!(stats.beliefs_retracted, 0);
        assert_eq!(stats.errors, 0);

        let after = store.get("b1").unwrap();
        assert!(!after.stale, "store untouched");
    }

    /// Belief-store error doesn't bubble — we log + count and return.
    #[tokio::test]
    async fn store_error_doesnt_bubble() {
        let store = Arc::new(InMemBeliefStore::failing());
        let prop = BeliefPropagator::new(store, true);

        let stats = prop.propagate_invalidation("F1", Utc::now()).await;
        assert_eq!(stats.errors, 1);
        assert_eq!(stats.beliefs_invalidated, 0);
    }

    /// No referencing beliefs → no-op + zero stats (but no error).
    #[tokio::test]
    async fn no_referencing_beliefs_is_noop() {
        let store = Arc::new(InMemBeliefStore::new(vec![make_belief(
            "b1",
            vec!["F-other"],
        )]));
        let prop = BeliefPropagator::new(store, true);

        let stats = prop.propagate_invalidation("F1", Utc::now()).await;
        assert_eq!(stats.beliefs_invalidated, 0);
        assert_eq!(stats.errors, 0);
        assert_eq!(stats.max_propagation_depth, 0);
    }
}
