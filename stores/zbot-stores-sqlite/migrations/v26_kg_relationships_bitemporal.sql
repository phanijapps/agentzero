-- stores/zbot-stores-sqlite/migrations/v26_kg_relationships_bitemporal.sql
--
-- Bi-temporal phase 3: align `kg_relationships` with the
-- `memory_facts` / `kg_entities` schema by introducing the symmetric
-- `valid_from` / `valid_until` pair. The legacy `valid_at` /
-- `invalidated_at` columns are KEPT for now — older writers continue to
-- populate them during the gradual transition. A future migration will
-- drop them once an audit confirms no readers depend on them.
--
-- The `ALTER TABLE ADD COLUMN` step for valid_from / valid_until is
-- handled by `ensure_kg_relationships_bitemporal_columns` in
-- `knowledge_schema.rs`, which uses the same PRAGMA-guarded pattern as
-- `ensure_evidence_column` so the migration is re-runnable on already
-- migrated databases (SQLite errors on duplicate ADD COLUMN; raw SQL
-- in this file cannot guard the ALTER). By the time the UPDATE
-- statements below run, both columns are guaranteed to exist.
--
-- The UPDATE statements themselves are naturally idempotent: they only
-- touch rows where the new column is still NULL, so re-runs are no-ops.

UPDATE kg_relationships
   SET valid_from = valid_at
 WHERE valid_from IS NULL AND valid_at IS NOT NULL;

UPDATE kg_relationships
   SET valid_until = invalidated_at
 WHERE valid_until IS NULL AND invalidated_at IS NOT NULL;
