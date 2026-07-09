//! Belief Network observability endpoints (Phase B-6).
//!
//! Surfaces what the sleep-time belief-network workers are doing via two
//! GET handlers:
//!
//! - `GET /api/belief-network/stats`     — latest + recent cycle stats per
//!   worker, plus aggregate belief / contradiction totals.
//! - `GET /api/belief-network/activity`  — per-belief activity events
//!   derived from `kg_beliefs` and `kg_belief_contradictions`.
//!
//! Disabled gracefully when `beliefNetwork.enabled = false`: both endpoints
//! return empty payloads with `enabled: false`, never `503`. The activity
//! list is derived from existing tables — no new activity log is written.

use crate::state::AppState;
use axum::{
    extract::{Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use gateway_memory::{
    BeliefPropagationStats, BeliefSynthesisStats, ContradictionDetectionStats,
    RecentBeliefNetworkActivity, TimestampedContradictionStats, TimestampedPropagationStats,
    TimestampedSynthesisStats,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use zbot_stores_domain::{BeliefContradiction, ContradictionType, Resolution};

// ============================================================================
// CONSTANTS
// ============================================================================

/// `partition_id` used for belief lookups. Today every belief is bucketed
/// under the root agent (mirrors `MemoryServicesConfig.agent_id` in
/// `state/mod.rs`). When/if ward-scoped beliefs land, this becomes a
/// query parameter.
const DEFAULT_PARTITION: &str = "root";

/// Default `limit` for `GET /api/belief-network/activity`.
const DEFAULT_ACTIVITY_LIMIT: usize = 50;

/// Maximum `limit` for `GET /api/belief-network/activity` (defensive cap).
const MAX_ACTIVITY_LIMIT: usize = 200;

/// Pull window used to bound the in-memory queries that back the activity
/// feed. Picking a generous overscan (`4 ×` requested) so the merged
/// timeline still has enough rows to fill the response when one stream
/// is much busier than the other.
const ACTIVITY_PULL_MULTIPLIER: usize = 4;

// ============================================================================
// STATS RESPONSE
// ============================================================================

/// Wire shape for `GET /api/belief-network/stats`.
#[derive(Debug, Serialize)]
pub struct BeliefNetworkStatsResponse {
    pub enabled: bool,
    pub synthesizer: WorkerStats<BeliefSynthesisStatsDto, BeliefSynthesisHistoryEntry>,
    pub contradiction_detector:
        WorkerStats<ContradictionDetectionStatsDto, ContradictionHistoryEntry>,
    pub propagator: WorkerStats<BeliefPropagationStatsDto, PropagationHistoryEntry>,
    pub totals: BeliefNetworkTotals,
}

/// Per-worker stats payload: most-recent snapshot + cycle history.
#[derive(Debug, Serialize)]
pub struct WorkerStats<L, H> {
    pub latest: L,
    pub history: Vec<H>,
}

/// Aggregate counts pulled directly from `kg_beliefs` /
/// `kg_belief_contradictions`.
#[derive(Debug, Default, Serialize)]
pub struct BeliefNetworkTotals {
    pub total_beliefs: usize,
    pub total_contradictions: usize,
    pub total_unresolved_contradictions: usize,
}

#[derive(Debug, Default, Serialize)]
pub struct BeliefSynthesisStatsDto {
    pub subjects_examined: u64,
    pub beliefs_synthesized: u64,
    pub beliefs_short_circuited: u64,
    pub beliefs_llm_synthesized: u64,
    pub llm_calls: u64,
    pub errors: u64,
    pub stale_beliefs_resynthesized: u64,
}

impl From<&BeliefSynthesisStats> for BeliefSynthesisStatsDto {
    fn from(s: &BeliefSynthesisStats) -> Self {
        Self {
            subjects_examined: s.subjects_examined,
            beliefs_synthesized: s.beliefs_synthesized,
            beliefs_short_circuited: s.beliefs_short_circuited,
            beliefs_llm_synthesized: s.beliefs_llm_synthesized,
            llm_calls: s.llm_calls,
            errors: s.errors,
            stale_beliefs_resynthesized: s.stale_beliefs_resynthesized,
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct ContradictionDetectionStatsDto {
    pub neighborhoods_examined: u64,
    pub pairs_examined: u64,
    pub pairs_skipped_existing: u64,
    pub llm_calls: u64,
    pub contradictions_logical: u64,
    pub contradictions_tension: u64,
    pub duplicates_logged: u64,
    pub compatibles_logged: u64,
    pub errors: u64,
    pub budget_exhausted: bool,
}

impl From<&ContradictionDetectionStats> for ContradictionDetectionStatsDto {
    fn from(s: &ContradictionDetectionStats) -> Self {
        Self {
            neighborhoods_examined: s.neighborhoods_examined,
            pairs_examined: s.pairs_examined,
            pairs_skipped_existing: s.pairs_skipped_existing,
            llm_calls: s.llm_calls,
            contradictions_logical: s.contradictions_logical,
            contradictions_tension: s.contradictions_tension,
            duplicates_logged: s.duplicates_logged,
            compatibles_logged: s.compatibles_logged,
            errors: s.errors,
            budget_exhausted: s.budget_exhausted,
        }
    }
}

#[derive(Debug, Default, Serialize)]
pub struct BeliefPropagationStatsDto {
    pub beliefs_invalidated: u64,
    pub beliefs_retracted: u64,
    pub beliefs_marked_stale: u64,
    pub max_propagation_depth: u32,
    pub errors: u64,
}

impl From<&BeliefPropagationStats> for BeliefPropagationStatsDto {
    fn from(s: &BeliefPropagationStats) -> Self {
        Self {
            beliefs_invalidated: s.beliefs_invalidated,
            beliefs_retracted: s.beliefs_retracted,
            beliefs_marked_stale: s.beliefs_marked_stale,
            max_propagation_depth: s.max_propagation_depth,
            errors: s.errors,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct BeliefSynthesisHistoryEntry {
    pub timestamp: String,
    #[serde(flatten)]
    pub stats: BeliefSynthesisStatsDto,
}

#[derive(Debug, Serialize)]
pub struct ContradictionHistoryEntry {
    pub timestamp: String,
    #[serde(flatten)]
    pub stats: ContradictionDetectionStatsDto,
}

#[derive(Debug, Serialize)]
pub struct PropagationHistoryEntry {
    pub timestamp: String,
    #[serde(flatten)]
    pub stats: BeliefPropagationStatsDto,
}

// ============================================================================
// ACTIVITY RESPONSE
// ============================================================================

/// Query parameters for `GET /api/belief-network/activity`.
#[derive(Debug, Deserialize)]
pub struct ActivityQuery {
    pub limit: Option<usize>,
}

/// One activity event in the reverse-chronological feed.
#[derive(Debug, Serialize)]
pub struct BeliefActivityEvent {
    pub kind: BeliefActivityKind,
    pub timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub belief_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub summary: String,
}

#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum BeliefActivityKind {
    Synthesized,
    Retracted,
    MarkedStale,
    ContradictionDetected,
    ContradictionResolved,
    PropagationCascade,
}

// ============================================================================
// HANDLERS
// ============================================================================

/// `GET /api/belief-network/stats`
pub async fn get_stats(State(state): State<AppState>) -> Json<BeliefNetworkStatsResponse> {
    let enabled = belief_network_enabled(&state);

    let activity = state.belief_network_activity.clone();
    let synthesizer_stats = build_synthesizer_stats(activity.as_ref());
    let contradiction_stats = build_contradiction_stats(activity.as_ref());
    let propagator_stats = build_propagator_stats(activity.as_ref());

    let totals = if enabled {
        compute_totals(&state).await
    } else {
        BeliefNetworkTotals::default()
    };

    Json(BeliefNetworkStatsResponse {
        enabled,
        synthesizer: synthesizer_stats,
        contradiction_detector: contradiction_stats,
        propagator: propagator_stats,
        totals,
    })
}

/// `GET /api/belief-network/activity`
pub async fn get_activity(
    Query(query): Query<ActivityQuery>,
    State(state): State<AppState>,
) -> Json<Vec<BeliefActivityEvent>> {
    let limit = query
        .limit
        .unwrap_or(DEFAULT_ACTIVITY_LIMIT)
        .clamp(1, MAX_ACTIVITY_LIMIT);

    if !belief_network_enabled(&state) {
        return Json(Vec::new());
    }

    let mut events = Vec::new();
    let pull = limit.saturating_mul(ACTIVITY_PULL_MULTIPLIER);

    if let Some(store) = state.belief_store.as_ref() {
        push_belief_events(store, pull, &mut events).await;
    }
    if let Some(store) = state.belief_contradiction_store.as_ref() {
        push_contradiction_events(store, pull, &mut events).await;
    }

    // Reverse-chronological ordering with `limit` truncation.
    events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    events.truncate(limit);
    Json(events)
}

// ============================================================================
// HELPERS — stats
// ============================================================================

fn belief_network_enabled(state: &AppState) -> bool {
    state
        .settings
        .get_execution_settings()
        .map(|s| s.memory.belief_network.enabled)
        .unwrap_or(false)
}

fn build_synthesizer_stats(
    activity: Option<&Arc<RecentBeliefNetworkActivity>>,
) -> WorkerStats<BeliefSynthesisStatsDto, BeliefSynthesisHistoryEntry> {
    let history: Vec<TimestampedSynthesisStats> =
        activity.map(|a| a.synthesis_history()).unwrap_or_default();
    let latest = history
        .last()
        .map(|e| BeliefSynthesisStatsDto::from(&e.stats))
        .unwrap_or_default();
    let history_dto = history
        .into_iter()
        .map(|e| BeliefSynthesisHistoryEntry {
            timestamp: e.timestamp.to_rfc3339(),
            stats: BeliefSynthesisStatsDto::from(&e.stats),
        })
        .collect();
    WorkerStats {
        latest,
        history: history_dto,
    }
}

fn build_contradiction_stats(
    activity: Option<&Arc<RecentBeliefNetworkActivity>>,
) -> WorkerStats<ContradictionDetectionStatsDto, ContradictionHistoryEntry> {
    let history: Vec<TimestampedContradictionStats> = activity
        .map(|a| a.contradiction_history())
        .unwrap_or_default();
    let latest = history
        .last()
        .map(|e| ContradictionDetectionStatsDto::from(&e.stats))
        .unwrap_or_default();
    let history_dto = history
        .into_iter()
        .map(|e| ContradictionHistoryEntry {
            timestamp: e.timestamp.to_rfc3339(),
            stats: ContradictionDetectionStatsDto::from(&e.stats),
        })
        .collect();
    WorkerStats {
        latest,
        history: history_dto,
    }
}

fn build_propagator_stats(
    activity: Option<&Arc<RecentBeliefNetworkActivity>>,
) -> WorkerStats<BeliefPropagationStatsDto, PropagationHistoryEntry> {
    let history: Vec<TimestampedPropagationStats> = activity
        .map(|a| a.propagation_history())
        .unwrap_or_default();
    let latest = history
        .last()
        .map(|e| BeliefPropagationStatsDto::from(&e.stats))
        .unwrap_or_default();
    let history_dto = history
        .into_iter()
        .map(|e| PropagationHistoryEntry {
            timestamp: e.timestamp.to_rfc3339(),
            stats: BeliefPropagationStatsDto::from(&e.stats),
        })
        .collect();
    WorkerStats {
        latest,
        history: history_dto,
    }
}

async fn compute_totals(state: &AppState) -> BeliefNetworkTotals {
    let mut totals = BeliefNetworkTotals::default();

    if let Some(store) = state.belief_store.as_ref() {
        // The belief population is bounded by design — a generous cap
        // mirrors the historical pattern in `graph::graph_stats`.
        if let Ok(beliefs) = store.list_beliefs(DEFAULT_PARTITION, 100_000).await {
            totals.total_beliefs = beliefs.len();
        }
    }

    if let Some(store) = state.belief_contradiction_store.as_ref() {
        if let Ok(rows) = store.list_recent(DEFAULT_PARTITION, 100_000).await {
            totals.total_contradictions = rows.len();
            totals.total_unresolved_contradictions =
                rows.iter().filter(|c| !is_resolved(c)).count();
        }
    }

    totals
}

fn is_resolved(c: &BeliefContradiction) -> bool {
    matches!(
        c.resolution,
        Some(Resolution::AWon) | Some(Resolution::BWon) | Some(Resolution::Compatible)
    )
}

// ============================================================================
// HELPERS — activity
// ============================================================================

async fn push_belief_events(
    store: &Arc<dyn zbot_stores::BeliefStore>,
    pull: usize,
    out: &mut Vec<BeliefActivityEvent>,
) {
    let beliefs = match store.list_beliefs(DEFAULT_PARTITION, pull).await {
        Ok(rows) => rows,
        Err(_) => return,
    };

    for b in beliefs {
        out.push(belief_to_event(&b));
        if let Some(retracted_at) = b.valid_until {
            // A retracted (or superseded) belief surfaces a second event
            // — its retraction timestamp is the more relevant signal for
            // the activity feed.
            out.push(BeliefActivityEvent {
                kind: classify_termination(&b),
                timestamp: retracted_at.to_rfc3339(),
                belief_id: Some(b.id.clone()),
                subject: Some(b.subject.clone()),
                summary: termination_summary(&b),
            });
        }
        if b.stale {
            out.push(BeliefActivityEvent {
                kind: BeliefActivityKind::MarkedStale,
                timestamp: b.updated_at.to_rfc3339(),
                belief_id: Some(b.id.clone()),
                subject: Some(b.subject.clone()),
                summary: format!("Belief about \"{}\" marked stale", b.subject),
            });
            // Propagation depth > 1 is the placeholder for B-6+; for
            // now `max_propagation_depth = 1` so we don't emit a
            // separate cascade event per stale belief.
        }
    }
}

async fn push_contradiction_events(
    store: &Arc<dyn zbot_stores::BeliefContradictionStore>,
    pull: usize,
    out: &mut Vec<BeliefActivityEvent>,
) {
    let rows = match store.list_recent(DEFAULT_PARTITION, pull).await {
        Ok(rs) => rs,
        Err(_) => return,
    };

    for c in rows {
        out.push(BeliefActivityEvent {
            kind: BeliefActivityKind::ContradictionDetected,
            timestamp: c.detected_at.to_rfc3339(),
            belief_id: Some(c.belief_a_id.clone()),
            subject: None,
            summary: format!(
                "Contradiction detected ({}) between {} and {}",
                contradiction_type_label(&c.contradiction_type),
                c.belief_a_id,
                c.belief_b_id
            ),
        });
        if let (Some(resolved_at), true) = (c.resolved_at, is_resolved(&c)) {
            out.push(BeliefActivityEvent {
                kind: BeliefActivityKind::ContradictionResolved,
                timestamp: resolved_at.to_rfc3339(),
                belief_id: Some(c.belief_a_id.clone()),
                subject: None,
                summary: format!(
                    "Contradiction resolved ({})",
                    resolution_label(c.resolution.as_ref())
                ),
            });
        }
    }
}

fn belief_to_event(b: &zbot_stores_domain::Belief) -> BeliefActivityEvent {
    BeliefActivityEvent {
        kind: BeliefActivityKind::Synthesized,
        timestamp: b.created_at.to_rfc3339(),
        belief_id: Some(b.id.clone()),
        subject: Some(b.subject.clone()),
        summary: format!(
            "Belief synthesized for \"{}\" ({} fact{})",
            b.subject,
            b.source_fact_ids.len(),
            if b.source_fact_ids.len() == 1 {
                ""
            } else {
                "s"
            }
        ),
    }
}

fn classify_termination(b: &zbot_stores_domain::Belief) -> BeliefActivityKind {
    if b.superseded_by.is_some() {
        BeliefActivityKind::PropagationCascade
    } else {
        BeliefActivityKind::Retracted
    }
}

fn termination_summary(b: &zbot_stores_domain::Belief) -> String {
    if b.superseded_by.is_some() {
        format!("Belief about \"{}\" superseded", b.subject)
    } else {
        format!("Belief about \"{}\" retracted", b.subject)
    }
}

fn contradiction_type_label(t: &ContradictionType) -> &'static str {
    match t {
        ContradictionType::Logical => "logical",
        ContradictionType::Tension => "tension",
        ContradictionType::Temporal => "temporal",
    }
}

fn resolution_label(r: Option<&Resolution>) -> &'static str {
    match r {
        Some(Resolution::AWon) => "a_won",
        Some(Resolution::BWon) => "b_won",
        Some(Resolution::Compatible) => "compatible",
        _ => "unresolved",
    }
}

// Re-export DateTime helper alias so future changes don't drift.
#[allow(dead_code)]
type _RfcAlias = DateTime<Utc>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_resolved_handles_each_variant() {
        let mut c = sample_contradiction();
        c.resolution = None;
        assert!(!is_resolved(&c));
        c.resolution = Some(Resolution::Unresolved);
        assert!(!is_resolved(&c));
        c.resolution = Some(Resolution::AWon);
        assert!(is_resolved(&c));
        c.resolution = Some(Resolution::BWon);
        assert!(is_resolved(&c));
        c.resolution = Some(Resolution::Compatible);
        assert!(is_resolved(&c));
    }

    #[test]
    fn contradiction_type_label_covers_all_variants() {
        assert_eq!(
            contradiction_type_label(&ContradictionType::Logical),
            "logical"
        );
        assert_eq!(
            contradiction_type_label(&ContradictionType::Tension),
            "tension"
        );
        assert_eq!(
            contradiction_type_label(&ContradictionType::Temporal),
            "temporal"
        );
    }

    #[test]
    fn resolution_label_falls_back_to_unresolved() {
        assert_eq!(resolution_label(None), "unresolved");
        assert_eq!(
            resolution_label(Some(&Resolution::Unresolved)),
            "unresolved"
        );
        assert_eq!(resolution_label(Some(&Resolution::AWon)), "a_won");
    }

    #[test]
    fn build_synthesizer_stats_returns_empty_when_no_recorder() {
        let stats = build_synthesizer_stats(None);
        assert_eq!(stats.history.len(), 0);
        assert_eq!(stats.latest.beliefs_synthesized, 0);
    }

    #[test]
    fn build_synthesizer_stats_uses_last_entry_as_latest() {
        let recorder = Arc::new(RecentBeliefNetworkActivity::new());
        recorder.record_synthesis(BeliefSynthesisStats {
            beliefs_synthesized: 3,
            ..Default::default()
        });
        recorder.record_synthesis(BeliefSynthesisStats {
            beliefs_synthesized: 7,
            ..Default::default()
        });
        let stats = build_synthesizer_stats(Some(&recorder));
        assert_eq!(stats.history.len(), 2);
        assert_eq!(stats.latest.beliefs_synthesized, 7);
    }

    #[test]
    fn build_contradiction_stats_propagates_budget_exhausted_flag() {
        let recorder = Arc::new(RecentBeliefNetworkActivity::new());
        recorder.record_contradiction(ContradictionDetectionStats {
            budget_exhausted: true,
            pairs_examined: 25,
            ..Default::default()
        });
        let stats = build_contradiction_stats(Some(&recorder));
        assert!(stats.latest.budget_exhausted);
        assert_eq!(stats.latest.pairs_examined, 25);
    }

    #[test]
    fn classify_termination_distinguishes_supersede_from_retract() {
        let mut b = sample_belief();
        b.valid_until = Some(Utc::now());
        b.superseded_by = None;
        assert!(matches!(
            classify_termination(&b),
            BeliefActivityKind::Retracted
        ));
        b.superseded_by = Some("belief-new".to_string());
        assert!(matches!(
            classify_termination(&b),
            BeliefActivityKind::PropagationCascade
        ));
    }

    fn sample_contradiction() -> BeliefContradiction {
        BeliefContradiction {
            id: "c1".into(),
            belief_a_id: "a".into(),
            belief_b_id: "b".into(),
            contradiction_type: ContradictionType::Logical,
            severity: 0.8,
            judge_reasoning: None,
            detected_at: Utc::now(),
            resolved_at: None,
            resolution: None,
        }
    }

    fn sample_belief() -> zbot_stores_domain::Belief {
        zbot_stores_domain::Belief {
            id: "b1".into(),
            partition_id: "root".into(),
            subject: "user.location".into(),
            content: "lives in Berlin".into(),
            confidence: 0.9,
            valid_from: Some(Utc::now()),
            valid_until: None,
            source_fact_ids: vec!["f1".into()],
            synthesizer_version: 1,
            reasoning: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            superseded_by: None,
            stale: false,
            embedding: None,
        }
    }
}
