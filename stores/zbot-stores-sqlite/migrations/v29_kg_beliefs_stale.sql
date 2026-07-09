-- v29: Add `stale` column to kg_beliefs to support B-3 confidence propagation.
--
-- When a source fact is invalidated and the belief has multiple sources,
-- the belief is marked stale (stale = 1) instead of immediately retracted.
-- The next BeliefSynthesizer cycle picks up stale beliefs and re-synthesizes
-- them from the remaining valid source facts, clearing the flag.
--
-- Sole-source beliefs are retracted directly (valid_until set), not marked stale.
--
-- Idempotent: the `ALTER TABLE ADD COLUMN` runs only when the column is
-- absent (guarded by `ensure_kg_beliefs_stale_column` in knowledge_schema.rs,
-- mirroring the v26 PRAGMA pattern). The index uses IF NOT EXISTS.

CREATE INDEX IF NOT EXISTS idx_beliefs_stale ON kg_beliefs(stale) WHERE stale = 1;
