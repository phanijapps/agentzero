//! Hierarchical-memory observability endpoint (Phase H-3/H-4 follow-up).
//!
//! Surfaces what the `HierarchyBuilder` sleep-time worker has produced
//! via a single GET handler:
//!
//!   `GET /api/hierarchy/stats?top_n=10`
//!
//! Returns layer-by-layer entity counts, total inter-cluster edge
//! count, and the top-N aggregates by `member_count` (with their
//! LLM-synthesised names + descriptions) for the configured root
//! agent. Enough to render the Observatory pill + slideover without a
//! second round-trip.
//!
//! Disabled gracefully when `hierarchy.enabled = false`: the handler
//! returns `enabled: false` with empty payload, never `503` — same
//! pattern as `belief_network::get_stats`.

use crate::state::AppState;
use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use zbot_stores::HierarchySummary;

/// `agent_id` queried by the stats endpoint. Mirrors the root-agent
/// convention used everywhere else in the gateway (see also
/// `belief_network::DEFAULT_PARTITION`).
const DEFAULT_AGENT_ID: &str = "root";

/// Default number of top aggregates to return when the caller omits
/// `?top_n=`. Picked to comfortably fill a slideover's first screen
/// without blowing out the response size on graphs with hundreds of
/// aggregates.
const DEFAULT_TOP_N: usize = 10;

/// Hard cap on `top_n` so a malicious client can't ask for the entire
/// aggregate table.
const MAX_TOP_N: usize = 100;

/// Query string for `GET /api/hierarchy/stats`.
#[derive(Debug, Deserialize)]
pub struct HierarchyStatsQuery {
    #[serde(default)]
    pub top_n: Option<usize>,
}

/// Wire shape for `GET /api/hierarchy/stats`.
#[derive(Debug, Serialize)]
pub struct HierarchyStatsResponse {
    /// Mirror of `execution.memory.hierarchy.enabled` for the UI to
    /// pivot on. When `false`, the rest of the payload is the default
    /// empty `HierarchySummary` — the UI hides the pill rather than
    /// rendering an empty drawer.
    pub enabled: bool,
    pub agent_id: String,
    pub summary: HierarchySummary,
}

/// `GET /api/hierarchy/stats`
///
/// Returns layer counts + total inter-cluster relations + top-N
/// aggregates by member size for the root agent. Never errors with
/// `503` — when the hierarchy is disabled or the `kg_store` isn't
/// wired the handler returns `enabled: false` with empty fields.
pub async fn get_stats(
    State(state): State<AppState>,
    Query(query): Query<HierarchyStatsQuery>,
) -> Json<HierarchyStatsResponse> {
    let enabled = state
        .settings
        .get_execution_settings()
        .map(|s| s.memory.hierarchy.enabled)
        .unwrap_or(false);
    let top_n = query.top_n.unwrap_or(DEFAULT_TOP_N).min(MAX_TOP_N);

    let summary = match state.kg_store.as_ref() {
        Some(store) => store
            .hierarchy_summary(DEFAULT_AGENT_ID, top_n)
            .await
            .unwrap_or_else(|_| HierarchySummary::default()),
        None => HierarchySummary::default(),
    };

    Json(HierarchyStatsResponse {
        enabled,
        agent_id: DEFAULT_AGENT_ID.to_string(),
        summary,
    })
}
