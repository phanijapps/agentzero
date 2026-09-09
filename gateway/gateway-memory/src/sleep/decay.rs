//! DecayEngine — surfaces prune candidates from the knowledge graph.
//!
//! Phase 4 uses a lightweight orphan + age heuristic rather than full-graph
//! decay math: an entity is a candidate when it has no relationships and
//! its `last_seen_at` is older than `min_age_days`. Archival and
//! already-compressed entities are excluded by the underlying query.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use knowledge_graph::kg_trait::KnowledgeGraphStore;
use zbot_stores_traits::MemoryFactStore;

use crate::sleep::belief_propagator::{BeliefPropagationStats, BeliefPropagator};

/// Tuning knobs for the decay pass.
#[derive(Debug, Clone)]
pub struct DecayConfig {
    /// Only consider entities last seen more than this many days ago.
    pub min_age_days: i64,
    /// Upper bound on the number of candidates returned per pass.
    pub limit: usize,
}

impl Default for DecayConfig {
    fn default() -> Self {
        Self {
            min_age_days: 30,
            limit: 100,
        }
    }
}

/// Counts returned by [`DecayEngine::decay_kg_confidence`].
#[derive(Debug, Default, Clone)]
pub struct KgDecayStats {
    pub entities_decayed: u64,
    pub relationships_decayed: u64,
}

/// Counts returned by [`DecayEngine::propagate_fact_contradictions`] —
/// MEM-001 Part A. `episodes_processed` is the number of distinct
/// contradicted-fact episode ids the call walked; `entities_decayed` /
/// `relationships_decayed` count the KG rows whose confidence was
/// multiplicatively reduced. `errors` covers store-level failures
/// (logged-and-continued, not bubbled).
#[derive(Debug, Default, Clone)]
pub struct ContradictionPropagationStats {
    pub episodes_processed: u64,
    pub entities_decayed: u64,
    pub relationships_decayed: u64,
    pub errors: u64,
}

/// Tuning knobs for [`DecayEngine::propagate_fact_contradictions`].
///
/// `enabled = false` makes the call a no-op without touching the
/// store. `decay_factor` is the multiplicative coefficient applied to
/// `confidence` (e.g. `0.9` = 10% indirect-contradiction decay). The
/// `min_floor` is enforced by the SQL UPDATE so confidence never drops
/// below it.
///
/// `lookback_hours` bounds the `memory_facts.updated_at` window — the
/// caller picks "facts contradicted since the last cycle ran" by
/// passing `last_propagation_at` as `since` to the trait method;
/// this struct just carries the default lookback used at first run.
#[derive(Debug, Clone)]
pub struct ContradictionPropagationConfig {
    pub enabled: bool,
    pub decay_factor: f64,
    pub min_floor: f64,
    pub lookback_hours: i64,
}

impl Default for ContradictionPropagationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            decay_factor: 0.9,
            min_floor: 0.05,
            lookback_hours: 24,
        }
    }
}

/// A decayed entity slated for soft-deletion by the Pruner.
#[derive(Debug, Clone)]
pub struct PruneCandidate {
    pub entity_id: String,
    pub name: String,
    pub entity_type: String,
    pub reason: String,
}

/// Decay pass over the knowledge graph for a single agent.
///
/// Phase D3: trait-routed. Calls `kg_store.list_orphan_old_candidates`
/// which works on both backends — SQLite uses the orphan-age JOIN
/// against `kg_relationships`, Surreal uses a subquery against the
/// `relationship` edge table with server-side date arithmetic.
///
/// B-3: optionally carries a [`BeliefPropagator`] plus a confidence
/// threshold. When a fact-confidence drop is observed (either crossing
/// below `fact_confidence_drop_threshold` or dropping by more than that
/// fraction in a single cycle), the caller passes the affected fact ids
/// to [`DecayEngine::propagate_fact_confidence_drops`] which fires the
/// propagator for each. The propagator never bubbles errors.
pub struct DecayEngine {
    kg_store: Arc<dyn KnowledgeGraphStore>,
    config: DecayConfig,
    /// B-3: optional propagator. `None` when the Belief Network is
    /// disabled — fact decay still runs but skips the propagation call.
    belief_propagator: Option<Arc<BeliefPropagator>>,
    /// B-3: threshold matching `BeliefNetworkConfig.fact_confidence_drop_threshold`.
    /// A fact-confidence transition fires propagation when EITHER
    /// `new < threshold && old >= threshold` (crossed the floor) OR
    /// `old - new > threshold` (dropped by more than the threshold in
    /// one cycle).
    fact_confidence_drop_threshold: f64,
    /// MEM-001 Part A: optional fact store, used to list contradicted
    /// facts and read their `source_episode_id` values. `None` skips
    /// contradiction propagation entirely.
    fact_store: Option<Arc<dyn MemoryFactStore>>,
    /// MEM-001 Part A: contradiction propagation knobs (enabled flag,
    /// decay factor, floor, lookback). Defaults are safe — see
    /// [`ContradictionPropagationConfig::default`].
    contradiction_config: ContradictionPropagationConfig,
}

impl DecayEngine {
    pub fn new(kg_store: Arc<dyn KnowledgeGraphStore>, config: DecayConfig) -> Self {
        Self {
            kg_store,
            config,
            belief_propagator: None,
            fact_confidence_drop_threshold: 0.3,
            fact_store: None,
            contradiction_config: ContradictionPropagationConfig::default(),
        }
    }

    /// Builder-style: attach a [`BeliefPropagator`] and the configured
    /// drop threshold so fact-confidence drops fire B-3 propagation.
    /// Pass `None` to leave the engine in pre-B-3 behavior.
    #[must_use]
    pub fn with_belief_propagator(
        mut self,
        propagator: Option<Arc<BeliefPropagator>>,
        threshold: f64,
    ) -> Self {
        self.belief_propagator = propagator;
        self.fact_confidence_drop_threshold = threshold;
        self
    }

    /// Builder-style: attach a fact store + config so the engine can
    /// run MEM-001 Part A contradiction propagation. Without this,
    /// [`Self::propagate_fact_contradictions`] is a no-op.
    #[must_use]
    pub fn with_contradiction_propagation(
        mut self,
        fact_store: Option<Arc<dyn MemoryFactStore>>,
        config: ContradictionPropagationConfig,
    ) -> Self {
        self.fact_store = fact_store;
        self.contradiction_config = config;
        self
    }

    /// MEM-001 Part A — lookback used by the sleep cycle to bound the
    /// `memory_facts.updated_at > since` filter. Hours rather than a
    /// timestamp so callers can compute `now - lookback` once per
    /// cycle without leaking config out of the engine.
    pub fn contradiction_lookback_hours(&self) -> i64 {
        self.contradiction_config.lookback_hours
    }

    /// Should a fact-confidence transition trigger propagation? Either
    /// the new confidence crossed below the configured floor OR the
    /// drop in a single cycle exceeded that floor (sharp decay).
    fn should_propagate(&self, old_confidence: f64, new_confidence: f64) -> bool {
        let crossed_floor = new_confidence < self.fact_confidence_drop_threshold
            && old_confidence >= self.fact_confidence_drop_threshold;
        let sharp_drop = (old_confidence - new_confidence) > self.fact_confidence_drop_threshold;
        crossed_floor || sharp_drop
    }

    /// Propagate fact-confidence drops to dependent beliefs. Caller
    /// supplies `(fact_id, old_confidence, new_confidence)` tuples that
    /// came out of whatever decay path it ran (e.g. the SQLite-level
    /// `decay_stale_facts`).
    ///
    /// Per fact: if `should_propagate` matches, the [`BeliefPropagator`]
    /// is fired with the given `transition_time`. Aggregate stats from
    /// every propagation call are merged and returned. No-op when the
    /// propagator is absent (Belief Network disabled).
    pub async fn propagate_fact_confidence_drops(
        &self,
        drops: &[(String, f64, f64)],
        transition_time: DateTime<Utc>,
    ) -> BeliefPropagationStats {
        let mut agg = BeliefPropagationStats::default();
        let Some(propagator) = self.belief_propagator.as_ref() else {
            return agg;
        };
        for (fact_id, old_conf, new_conf) in drops {
            if !self.should_propagate(*old_conf, *new_conf) {
                continue;
            }
            let stats = propagator
                .propagate_invalidation(fact_id, transition_time)
                .await;
            agg.beliefs_invalidated += stats.beliefs_invalidated;
            agg.beliefs_retracted += stats.beliefs_retracted;
            agg.beliefs_marked_stale += stats.beliefs_marked_stale;
            agg.errors += stats.errors;
            agg.max_propagation_depth = agg.max_propagation_depth.max(stats.max_propagation_depth);
        }
        agg
    }

    /// MEM-001 Part A — propagate fact-level contradictions down to
    /// the KG entities and relationships that share the contradicted
    /// fact's `source_episode_id`.
    ///
    /// Steps per cycle:
    ///   1. List distinct `source_episode_id`s of `memory_facts` rows
    ///      where `contradicted_by IS NOT NULL` and `updated_at > since`.
    ///   2. Find KG entities + relationships whose `source_episode_ids`
    ///      blob contains any of those episode ids.
    ///   3. Apply `confidence = MAX(min_floor, confidence * decay_factor)`
    ///      to those rows.
    ///
    /// No-op when disabled, when `fact_store` is unwired, or when no
    /// facts have been contradicted in the window. Errors are logged
    /// and absorbed into the returned `errors` counter — the sleep
    /// cycle never aborts on this step.
    pub async fn propagate_fact_contradictions(
        &self,
        agent_id: &str,
        since: DateTime<Utc>,
    ) -> ContradictionPropagationStats {
        let mut stats = ContradictionPropagationStats::default();
        if !self.contradiction_config.enabled {
            return stats;
        }
        let Some(fact_store) = self.fact_store.as_ref() else {
            return stats;
        };

        let episode_ids = match fact_store
            .list_contradicted_fact_episode_ids(agent_id, since)
            .await
        {
            Ok(ids) => ids,
            Err(e) => {
                tracing::warn!(error = %e, "list_contradicted_fact_episode_ids failed");
                stats.errors += 1;
                return stats;
            }
        };
        if episode_ids.is_empty() {
            return stats;
        }
        stats.episodes_processed = episode_ids.len() as u64;

        let nodes = match self
            .kg_store
            .find_kg_nodes_by_episode_ids(agent_id, &episode_ids)
            .await
        {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(error = %e, "find_kg_nodes_by_episode_ids failed");
                stats.errors += 1;
                return stats;
            }
        };

        let factor = self.contradiction_config.decay_factor;
        let floor = self.contradiction_config.min_floor;

        match self
            .kg_store
            .apply_entity_confidence_multiplier(agent_id, &nodes.entity_ids, factor, floor)
            .await
        {
            Ok(n) => stats.entities_decayed = n,
            Err(e) => {
                tracing::warn!(error = %e, "apply_entity_confidence_multiplier failed");
                stats.errors += 1;
            }
        }
        match self
            .kg_store
            .apply_relationship_confidence_multiplier(
                agent_id,
                &nodes.relationship_ids,
                factor,
                floor,
            )
            .await
        {
            Ok(n) => stats.relationships_decayed = n,
            Err(e) => {
                tracing::warn!(error = %e, "apply_relationship_confidence_multiplier failed");
                stats.errors += 1;
            }
        }
        stats
    }

    /// Apply temporal confidence decay to KG entities and relationships.
    /// Conservative: errors are logged and the cycle returns whatever stats
    /// were collected before the failure.
    pub async fn decay_kg_confidence(
        &self,
        agent_id: &str,
        config: &crate::KgDecayConfig,
    ) -> KgDecayStats {
        let mut stats = KgDecayStats::default();
        if !config.enabled {
            return stats;
        }
        match self
            .kg_store
            .decay_entity_confidence(
                agent_id,
                config.entity_half_life_days,
                config.min_confidence,
                config.skip_recent_hours,
            )
            .await
        {
            Ok(n) => stats.entities_decayed = n,
            Err(e) => tracing::warn!(error = %e, "decay_entity_confidence failed"),
        }
        match self
            .kg_store
            .decay_relationship_confidence(
                agent_id,
                config.relationship_half_life_days,
                config.min_confidence,
                config.skip_recent_hours,
            )
            .await
        {
            Ok(n) => stats.relationships_decayed = n,
            Err(e) => tracing::warn!(error = %e, "decay_relationship_confidence failed"),
        }
        stats
    }

    /// Return prune candidates for `agent_id`. On query failure, returns an
    /// empty vec (the sleep worker treats decay as best-effort).
    pub async fn list_prune_candidates(&self, agent_id: &str) -> Vec<PruneCandidate> {
        match self
            .kg_store
            .list_orphan_old_candidates(agent_id, self.config.min_age_days, self.config.limit)
            .await
        {
            Ok(rows) => rows
                .into_iter()
                .map(|c| PruneCandidate {
                    entity_id: c.id,
                    name: c.name,
                    entity_type: c.entity_type,
                    reason: format!(
                        "orphan age>{}d mention_count={}",
                        self.config.min_age_days, c.mention_count
                    ),
                })
                .collect(),
            Err(e) => {
                tracing::warn!(error = %e, "list_orphan_old_candidates failed");
                Vec::new()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sleep::test_support;
    use knowledge_graph::kg_trait::KnowledgeGraphStore;
    use knowledge_graph::{Entity, EntityType, ExtractedKnowledge, Relationship, RelationshipType};
    use std::sync::Arc;

    fn setup() -> (tempfile::TempDir, Arc<dyn KnowledgeGraphStore>) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let kg = test_support::kg_store(&tmp);
        (tmp, kg)
    }

    /// Sqlite KG reference-implementation fixture for the decay /
    /// contradiction-propagation integration tests (see note above
    /// `insert_kg_entity_with_episode`).
    fn sqlite_graph() -> (
        tempfile::TempDir,
        Arc<zbot_stores_sqlite::kg::storage::GraphStorage>,
    ) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let paths = Arc::new(agent_primitives::vault_paths::VaultPaths::new(
            tmp.path().to_path_buf(),
        ));
        std::fs::create_dir_all(paths.conversations_db().parent().expect("parent")).expect("mkdir");
        let db = Arc::new(zbot_stores_sqlite::KnowledgeDatabase::new(paths).expect("db"));
        let graph =
            Arc::new(zbot_stores_sqlite::kg::storage::GraphStorage::new(db).expect("graph"));
        (tmp, graph)
    }

    #[tokio::test]
    async fn decay_engine_returns_only_orphan_old_non_archival() {
        let (_tmp, graph) = sqlite_graph();
        let agent_id = "agent-decay";

        // 1. An orphan, old entity -> should be returned.
        let mut orphan = Entity::new(
            agent_id.to_string(),
            EntityType::Concept,
            "Stale Topic".to_string(),
        );
        orphan.last_seen_at = chrono::Utc::now() - chrono::Duration::days(90);
        orphan.first_seen_at = orphan.last_seen_at;

        // 2. An entity with relationships -> should NOT be returned.
        let mut connected_a = Entity::new(
            agent_id.to_string(),
            EntityType::Person,
            "Active Alice".to_string(),
        );
        connected_a.last_seen_at = chrono::Utc::now() - chrono::Duration::days(90);
        connected_a.first_seen_at = connected_a.last_seen_at;
        let mut connected_b = Entity::new(
            agent_id.to_string(),
            EntityType::Organization,
            "Active Org".to_string(),
        );
        connected_b.last_seen_at = chrono::Utc::now() - chrono::Duration::days(90);
        connected_b.first_seen_at = connected_b.last_seen_at;

        let rel = Relationship::new(
            agent_id.to_string(),
            connected_a.id.clone(),
            connected_b.id.clone(),
            RelationshipType::WorksFor,
        );

        // 3. A recent orphan -> should NOT be returned (too young).
        let mut recent_orphan = Entity::new(
            agent_id.to_string(),
            EntityType::Concept,
            "Fresh Topic".to_string(),
        );
        recent_orphan.last_seen_at = chrono::Utc::now();
        recent_orphan.first_seen_at = recent_orphan.last_seen_at;

        graph
            .store_knowledge(
                agent_id,
                ExtractedKnowledge {
                    entities: vec![
                        orphan.clone(),
                        connected_a,
                        connected_b,
                        recent_orphan.clone(),
                    ],
                    relationships: vec![rel],
                },
            )
            .expect("store");

        let kg_store: Arc<dyn KnowledgeGraphStore> =
            Arc::new(zbot_stores_sqlite::SqliteKgStore::new(graph.clone()));
        let engine = DecayEngine::new(
            kg_store,
            DecayConfig {
                min_age_days: 30,
                limit: 100,
            },
        );
        let candidates = engine.list_prune_candidates(agent_id).await;

        let names: Vec<&str> = candidates.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"Stale Topic"),
            "expected stale orphan to be returned; got {names:?}"
        );
        assert!(
            !names.contains(&"Active Alice") && !names.contains(&"Active Org"),
            "connected entities should not be returned; got {names:?}"
        );
        assert!(
            !names.contains(&"Fresh Topic"),
            "recent entity should not be returned; got {names:?}"
        );
    }

    #[tokio::test]
    async fn decay_kg_confidence_returns_stats_when_enabled() {
        let (_tmp, graph) = sqlite_graph();
        let agent_id = "agent-kg-decay";

        // Seed one old entity (confidence is a KG-storage column; see the
        // sqlite-reference note above `insert_kg_entity_with_episode`.
        graph
            .knowledge_db()
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at)
                     VALUES ('old-1', ?1, 'Concept', 'Old', 'old', 'h1', 'current',
                             0.8, 1, 0, ?2, ?2)",
                    rusqlite::params![
                        agent_id,
                        (chrono::Utc::now() - chrono::Duration::days(180)).to_rfc3339()
                    ],
                )?;
                Ok(())
            })
            .unwrap();

        let kg_store: Arc<dyn KnowledgeGraphStore> =
            Arc::new(zbot_stores_sqlite::SqliteKgStore::new(graph));
        let engine = DecayEngine::new(kg_store, DecayConfig::default());
        let config = crate::KgDecayConfig::default();
        let stats = engine.decay_kg_confidence(agent_id, &config).await;
        assert_eq!(stats.entities_decayed, 1);
        assert_eq!(stats.relationships_decayed, 0);
    }

    #[tokio::test]
    async fn decay_kg_confidence_no_op_when_disabled() {
        let (_tmp, kg_store) = setup();
        let engine = DecayEngine::new(kg_store, DecayConfig::default());
        let config = crate::KgDecayConfig {
            enabled: false,
            ..Default::default()
        };
        let stats = engine.decay_kg_confidence("any", &config).await;
        assert_eq!(stats.entities_decayed, 0);
        assert_eq!(stats.relationships_decayed, 0);
    }

    // ------------------------------------------------------------------
    // B-3: fact-confidence drop propagation
    // ------------------------------------------------------------------

    /// A fact crossing below the configured threshold fires
    /// propagation. A fact whose new confidence stays above is ignored.
    /// A fact whose drop in one cycle exceeds the threshold also fires
    /// even if the new value is still above the floor.
    #[tokio::test]
    async fn propagate_fact_confidence_drops_threshold_logic() {
        use crate::sleep::belief_propagator::BeliefPropagator;
        use zbot_stores_traits::Belief;

        let (_tmp, kg_store) = setup();
        let (belief_store, _contradictions) = test_support::belief_stores(&_tmp);
        let now = chrono::Utc::now();
        // Two beliefs: one sourced from "F-crossing", one from
        // "F-stable". Only the first should be touched.
        let b_crossing = Belief {
            id: "b-crossing".into(),
            partition_id: "p".into(),
            subject: "user.x".into(),
            content: "c".into(),
            confidence: 0.8,
            valid_from: Some(now),
            valid_until: None,
            source_fact_ids: vec!["F-crossing".into()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding: None,
        };
        let b_stable = Belief {
            id: "b-stable".into(),
            partition_id: "p".into(),
            subject: "user.y".into(),
            content: "c".into(),
            confidence: 0.8,
            valid_from: Some(now),
            valid_until: None,
            source_fact_ids: vec!["F-stable".into()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding: None,
        };
        belief_store.upsert_belief(&b_crossing).await.unwrap();
        belief_store.upsert_belief(&b_stable).await.unwrap();

        let propagator = Arc::new(BeliefPropagator::new(belief_store.clone(), true));
        let engine = DecayEngine::new(kg_store, DecayConfig::default())
            .with_belief_propagator(Some(propagator), 0.3);

        // F-crossing: 0.5 → 0.2 (crosses below 0.3 floor)
        // F-stable: 0.9 → 0.8 (stays above floor and drop < 0.3)
        let drops = vec![
            ("F-crossing".to_string(), 0.5_f64, 0.2_f64),
            ("F-stable".to_string(), 0.9_f64, 0.8_f64),
        ];
        let stats = engine
            .propagate_fact_confidence_drops(&drops, chrono::Utc::now())
            .await;
        assert_eq!(
            stats.beliefs_invalidated, 1,
            "exactly one belief touched: {stats:?}"
        );
        assert_eq!(stats.beliefs_retracted, 1, "sole-source belief retracted");
    }

    /// Sharp drop in a single cycle (>threshold) fires propagation
    /// even when the new value still sits above the floor.
    #[tokio::test]
    async fn propagate_sharp_drop_fires_even_above_floor() {
        use crate::sleep::belief_propagator::BeliefPropagator;
        use zbot_stores_traits::Belief;

        let (_tmp, kg_store) = setup();
        let (belief_store, _contradictions) = test_support::belief_stores(&_tmp);
        let now = chrono::Utc::now();
        let b = Belief {
            id: "b-sharp".into(),
            partition_id: "p".into(),
            subject: "user.x".into(),
            content: "c".into(),
            confidence: 0.9,
            valid_from: Some(now),
            valid_until: None,
            source_fact_ids: vec!["F-sharp".into()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: now,
            updated_at: now,
            superseded_by: None,
            stale: false,
            embedding: None,
        };
        belief_store.upsert_belief(&b).await.unwrap();

        let propagator = Arc::new(BeliefPropagator::new(belief_store.clone(), true));
        let engine = DecayEngine::new(kg_store, DecayConfig::default())
            .with_belief_propagator(Some(propagator), 0.3);

        // 0.9 → 0.5: still above 0.3 floor but the drop (0.4) exceeds it.
        let drops = vec![("F-sharp".to_string(), 0.9_f64, 0.5_f64)];
        let stats = engine
            .propagate_fact_confidence_drops(&drops, chrono::Utc::now())
            .await;
        assert_eq!(stats.beliefs_invalidated, 1, "sharp-drop case fires");
    }

    /// Engine without a wired propagator is a no-op on fact-drop calls.
    #[tokio::test]
    async fn propagate_no_op_without_propagator() {
        let (_tmp, kg_store) = setup();
        let engine = DecayEngine::new(kg_store, DecayConfig::default());

        let drops = vec![("F-any".to_string(), 0.9_f64, 0.1_f64)];
        let stats = engine
            .propagate_fact_confidence_drops(&drops, chrono::Utc::now())
            .await;
        assert_eq!(stats.beliefs_invalidated, 0);
        assert_eq!(stats.errors, 0);
    }

    // ------------------------------------------------------------------
    // MEM-001 Part A: contradiction propagation
    // ------------------------------------------------------------------

    /// Seed a `memory_facts` row marked as contradicted. Uses the
    /// minimal columns the SQL path reads.
    async fn insert_contradicted_fact(
        fact_store: &dyn zbot_stores_traits::MemoryFactStore,
        fact_id: &str,
        agent_id: &str,
        source_episode_id: &str,
        contradicted_by: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        let fact = zbot_stores_domain::MemoryFact {
            id: fact_id.to_string(),
            session_id: None,
            agent_id: agent_id.to_string(),
            scope: "global".to_string(),
            category: "domain".to_string(),
            key: fact_id.to_string(),
            content: "c".to_string(),
            confidence: 0.8,
            mention_count: 1,
            source_summary: None,
            ward_id: "__global__".to_string(),
            contradicted_by: Some(contradicted_by.to_string()),
            created_at: now.clone(),
            updated_at: now,
            expires_at: None,
            valid_from: None,
            valid_until: None,
            superseded_by: None,
            pinned: false,
            epistemic_class: Some("current".to_string()),
            source_episode_id: Some(source_episode_id.to_string()),
            source_ref: None,
            last_accessed: None,
            embedding: None,
        };
        fact_store
            .upsert_typed_fact(fact, None)
            .await
            .expect("seed contradicted fact");
    }

    // KG-confidence decay + contradiction propagation surfaces exist only
    // on the sqlite KG store (production currently no-ops them through the
    // engram adapter's trait defaults). These integration tests stay on the
    // sqlite reference implementation until the KG lane migrates — tracked
    // as the "KG lane → engram" backlog item.
    fn insert_kg_entity_with_episode(
        graph: &zbot_stores_sqlite::kg::storage::GraphStorage,
        id: &str,
        agent_id: &str,
        confidence: f64,
        source_episode_ids: &str,
    ) {
        let now = chrono::Utc::now().to_rfc3339();
        graph
            .knowledge_db()
            .with_connection(|conn| {
                conn.execute(
                    "INSERT INTO kg_entities
                        (id, agent_id, entity_type, name, normalized_name, normalized_hash,
                         epistemic_class, confidence, mention_count, access_count,
                         first_seen_at, last_seen_at, source_episode_ids)
                     VALUES (?1, ?2, 'Concept', ?1, ?1, ?1, 'current', ?3, 1, 0, ?4, ?4, ?5)",
                    rusqlite::params![id, agent_id, confidence, now, source_episode_ids],
                )?;
                Ok(())
            })
            .unwrap();
    }

    #[tokio::test]
    async fn propagate_fact_contradictions_decays_kg_nodes_for_contradicted_episodes() {
        let (tmp, graph) = sqlite_graph();
        let agent = "a";
        let fact_store: Arc<dyn zbot_stores_traits::MemoryFactStore> =
            crate::sleep::test_support::fact_store(&tmp);

        // Two contradicted facts pointing to ep-1 and ep-2.
        insert_contradicted_fact(fact_store.as_ref(), "F1", agent, "ep-1", "F-newer").await;
        insert_contradicted_fact(fact_store.as_ref(), "F2", agent, "ep-2", "F-newer").await;

        // KG entities: e1 came from ep-1 (should decay), e2 from ep-99
        // (untouched), e3 from a multi-token blob including ep-2 (should
        // decay).
        insert_kg_entity_with_episode(&graph, "e1", agent, 0.8, "ep-1");
        insert_kg_entity_with_episode(&graph, "e2", agent, 0.8, "ep-99");
        insert_kg_entity_with_episode(&graph, "e3", agent, 0.8, "ep-2,ep-foo");

        let kg_store: Arc<dyn KnowledgeGraphStore> =
            Arc::new(zbot_stores_sqlite::SqliteKgStore::new(graph.clone()));

        let engine = DecayEngine::new(kg_store, DecayConfig::default())
            .with_contradiction_propagation(
                Some(fact_store),
                ContradictionPropagationConfig {
                    enabled: true,
                    decay_factor: 0.5,
                    min_floor: 0.05,
                    lookback_hours: 24,
                },
            );

        let stats = engine
            .propagate_fact_contradictions(agent, chrono::Utc::now() - chrono::Duration::days(7))
            .await;
        assert_eq!(stats.episodes_processed, 2);
        assert_eq!(stats.entities_decayed, 2);
        assert_eq!(stats.relationships_decayed, 0);
        assert_eq!(stats.errors, 0);

        let read = |id: &str| -> f64 {
            graph
                .knowledge_db()
                .with_connection(|conn| {
                    conn.query_row(
                        "SELECT confidence FROM kg_entities WHERE id = ?1",
                        rusqlite::params![id],
                        |row| row.get(0),
                    )
                })
                .unwrap()
        };
        assert!((read("e1") - 0.4).abs() < 1e-6, "e1: 0.8 * 0.5 = 0.4");
        assert!((read("e2") - 0.8).abs() < 1e-6, "e2 untouched");
        assert!((read("e3") - 0.4).abs() < 1e-6, "e3 decayed");
    }

    #[tokio::test]
    async fn propagate_fact_contradictions_disabled_is_noop() {
        let (_tmp, graph) = sqlite_graph();
        let kg_store: Arc<dyn KnowledgeGraphStore> =
            Arc::new(zbot_stores_sqlite::SqliteKgStore::new(graph));
        let engine = DecayEngine::new(kg_store, DecayConfig::default())
            .with_contradiction_propagation(
                None,
                ContradictionPropagationConfig {
                    enabled: false,
                    ..ContradictionPropagationConfig::default()
                },
            );
        let stats = engine
            .propagate_fact_contradictions("any", chrono::Utc::now())
            .await;
        assert_eq!(stats.episodes_processed, 0);
        assert_eq!(stats.entities_decayed, 0);
    }

    #[tokio::test]
    async fn propagate_fact_contradictions_no_factstore_is_noop() {
        let (_tmp, graph) = sqlite_graph();
        let kg_store: Arc<dyn KnowledgeGraphStore> =
            Arc::new(zbot_stores_sqlite::SqliteKgStore::new(graph));
        let engine = DecayEngine::new(kg_store, DecayConfig::default());
        let stats = engine
            .propagate_fact_contradictions("any", chrono::Utc::now())
            .await;
        assert_eq!(stats.episodes_processed, 0);
        assert_eq!(stats.errors, 0);
    }
}
