// ============================================================================
// Hierarchical-memory wire types
//
// Mirrors `gateway/src/http/hierarchy.rs::HierarchyStatsResponse` and
// the carriers re-exported from `zbot-stores`. Kept hand-written rather
// than codegen'd because the surface is tiny and the codebase doesn't
// run an OpenAPI codegen step yet.
// ============================================================================

export interface AggregateSummary {
  id: string;
  name: string;
  layer: number;
  member_count: number;
  description: string;
}

export interface HierarchySummary {
  /** `[layer, count]` pairs sorted ascending by layer. */
  layer_counts: Array<[number, number]>;
  /** Total `is_inter_cluster = 1` edges across all layers. */
  inter_cluster_relations: number;
  /** Top-N aggregates by member_count, descending. */
  top_aggregates: AggregateSummary[];
}

export interface HierarchyStatsResponse {
  /**
   * Mirror of `execution.memory.hierarchy.enabled`. When `false`,
   * `summary` carries the default-empty shape — the UI hides the
   * pill rather than rendering empty.
   */
  enabled: boolean;
  agent_id: string;
  summary: HierarchySummary;
}
