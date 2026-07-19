-- v31: Hierarchical memory columns on kg_entities + kg_relationships.
--
-- Lays the groundwork for HiRAG/LeanRAG-style hierarchical KG retrieval.
-- See `memory-bank/future-state/` (forthcoming) and the project memory
-- entry `project_hierarchical_memory_plan.md` for the full design.
--
-- The future HierarchyBuilder sleep worker (Phase H-3) will:
--   * GMM/K-means cluster layer-N entities by `kg_name_index` embeddings,
--   * LLM-summarise each cluster into an aggregate entity at layer N+1
--     written to this same `kg_entities` table (with `layer > 0`),
--   * link cluster members to their aggregate via `parent_cluster_id`,
--   * synthesise inter-cluster relations at layer N+1 with
--     `is_inter_cluster = 1` when the underlying connectivity exceeds
--     a threshold τ (LeanRAG's load-bearing innovation).
--
-- The future recall path (Phase H-4) walks `parent_cluster_id` upward
-- from top-N seed entities to their Lowest Common Ancestor and returns
-- the path subgraph + inter-cluster relations at the path's layers.
--
-- Both columns + the per-relationship `is_inter_cluster` flag default
-- to safe "no hierarchy yet" values (`layer = 0`, `parent_cluster_id = NULL`,
-- `is_inter_cluster = 0`), so existing rows behave identically to before.
-- No backfill required — every existing entity is a layer-0 base node
-- by definition, and every existing relationship is a layer-0 base edge.
--
-- Idempotent: the `ALTER TABLE ADD COLUMN` runs only when the columns
-- are absent (guarded by `ensure_kg_entities_hierarchy_columns` and
-- `ensure_kg_relationships_hierarchy_columns` in `knowledge_schema.rs`,
-- mirroring the v29/v30 PRAGMA pattern). Fresh databases get the
-- columns via the inline `CREATE TABLE` body. The indexes below use
-- `IF NOT EXISTS`.

CREATE INDEX IF NOT EXISTS idx_kg_entities_layer
    ON kg_entities(agent_id, layer);

CREATE INDEX IF NOT EXISTS idx_kg_entities_parent_cluster
    ON kg_entities(parent_cluster_id)
    WHERE parent_cluster_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_kg_relationships_layer
    ON kg_relationships(agent_id, layer);

CREATE INDEX IF NOT EXISTS idx_kg_relationships_inter_cluster
    ON kg_relationships(agent_id, layer)
    WHERE is_inter_cluster = 1;
